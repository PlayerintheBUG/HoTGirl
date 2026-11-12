package app.hotgirl

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.net.wifi.WifiManager
import android.os.IBinder
import android.os.PowerManager
import android.os.SystemClock
import android.widget.Toast
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

class HeadTrackerService : Service(), SensorEventListener {

    private lateinit var sensorManager: SensorManager
    private var socket: DatagramSocket? = null
    private var target: AtomicReference<InetAddress?> = AtomicReference(null)
    private var targetPort: Int = DEFAULT_PORT
    private val seq = AtomicInteger(0)
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    val running = AtomicBoolean(false)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        val host = intent?.getStringExtra(EXTRA_HOST) ?: return START_NOT_STICKY
        startForegroundNotification()
        try {
            targetPort = intent.getIntExtra(EXTRA_PORT, DEFAULT_PORT)
            target.set(InetAddress.getByName(host))

            sensorManager = getSystemService(SENSOR_SERVICE) as SensorManager
            val sensor = sensorManager.getDefaultSensor(Sensor.TYPE_GAME_ROTATION_VECTOR)
                ?: throw IllegalStateException("GAME_ROTATION_VECTOR non disponibile")

            val pm = getSystemService(POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hotgirl:tracker").apply {
                acquire()
            }
            val wm = applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
            wifiLock =
                wm.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "hotgirl:wifi").apply {
                    acquire()
                }

            socket?.close()
            socket = DatagramSocket()
            Thread(::pingLoop, "hotgirl-ping").start()

            sensorManager.registerListener(this, sensor, 10_000)
        } catch (e: Exception) {
            Toast.makeText(this, "HoTGirl: ${e.javaClass.simpleName}: ${e.message}", Toast.LENGTH_LONG)
                .show()
            lastError.set("start: ${e.javaClass.simpleName}: ${e.message}")
            stopSelf()
            return START_NOT_STICKY
        }
        running.set(true)
        lastError.set(null)
        Toast.makeText(this, "Invio a $host:$targetPort", Toast.LENGTH_SHORT).show()
        return START_STICKY
    }

    override fun onDestroy() {
        running.set(false)
        sensorManager.unregisterListener(this)
        wakeLock?.takeIf { it.isHeld }?.release()
        wifiLock?.takeIf { it.isHeld }?.release()
        socket?.close()
        socket = null
        super.onDestroy()
    }

    override fun onSensorChanged(event: SensorEvent) {
        val addr = target.get() ?: return
        val sock = socket ?: return
        val quaternion = event.values
        val buf = ByteArray(SAMPLE_PACKET_LEN)
        writeHeader(buf, KIND_SAMPLE, seq.getAndIncrement() and 0xFFFF)
        writeI64(buf, 16, event.timestamp)
        writeF32(buf, 24, quaternion[0])
        writeF32(buf, 28, quaternion[1])
        writeF32(buf, 32, quaternion[2])
        writeF32(buf, 36, quaternion[3])
        writeF32(buf, 40, event.accuracy.toFloat())
        val packet = DatagramPacket(buf, buf.size, addr, targetPort)
        try {
            sock.send(packet)
            samplesSent.incrementAndGet()
        } catch (e: Exception) {
            sendErrors.incrementAndGet()
            lastError.set("send: ${e.javaClass.simpleName}: ${e.message}")
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    private fun pingLoop() {
        val sock = socket ?: return
        val buf = ByteArray(64)
        while (!sock.isClosed) {
            val packet = DatagramPacket(buf, buf.size)
            try {
                sock.receive(packet)
            } catch (_: Exception) {
                continue
            }
            if (packet.length < HEADER_LEN) continue
            val kind = buf[5].toInt() and 0xFF
            if (kind != KIND_PING) continue
            val tPc = readI64(buf, 16)
            val reply = ByteArray(PONG_PACKET_LEN)
            writeHeader(reply, KIND_PONG, 0)
            writeI64(reply, 16, tPc)
            writeI64(reply, 24, SystemClock.elapsedRealtimeNanos())
            try {
                sock.send(DatagramPacket(reply, reply.size, packet.address, packet.port))
            } catch (_: Exception) {
            }
        }
    }

    private fun startForegroundNotification() {
        val channel =
            NotificationChannel(CHANNEL_ID, "Head tracker", NotificationManager.IMPORTANCE_LOW)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        val notification: Notification =
            Notification.Builder(this, CHANNEL_ID)
                .setContentTitle(getString(R.string.notification_title))
                .setSmallIcon(android.R.drawable.ic_menu_compass)
                .build()
        startForeground(
            NOTIFICATION_ID,
            notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        )
    }

    companion object {
        const val ACTION_STOP = "app.hotgirl.STOP"
        const val EXTRA_HOST = "host"
        const val EXTRA_PORT = "port"
        const val DEFAULT_PORT = 5555

        const val CHANNEL_ID = "hotgirl"
        const val NOTIFICATION_ID = 1

        val samplesSent = AtomicLong(0)
        val sendErrors = AtomicLong(0)
        val lastError = AtomicReference<String?>(null)

        const val MAGIC = 0x48544752
        const val KIND_SAMPLE = 1
        const val KIND_PING = 2
        const val KIND_PONG = 3
        const val HEADER_LEN = 16
        const val SAMPLE_PACKET_LEN = 44
        const val PONG_PACKET_LEN = 32

        fun start(context: Context, host: String, port: Int) {
            val intent = Intent(context, HeadTrackerService::class.java)
                .putExtra(EXTRA_HOST, host)
                .putExtra(EXTRA_PORT, port)
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            context.startService(
                Intent(context, HeadTrackerService::class.java).setAction(ACTION_STOP)
            )
        }

        fun writeHeader(buf: ByteArray, kind: Int, seq: Int) {
            writeU32(buf, 0, MAGIC)
            buf[4] = 1
            buf[5] = kind.toByte()
            buf[6] = (seq and 0xFF).toByte()
            buf[7] = ((seq shr 8) and 0xFF).toByte()
        }

        fun writeU32(buf: ByteArray, off: Int, v: Int) {
            buf[off] = (v and 0xFF).toByte()
            buf[off + 1] = ((v shr 8) and 0xFF).toByte()
            buf[off + 2] = ((v shr 16) and 0xFF).toByte()
            buf[off + 3] = ((v shr 24) and 0xFF).toByte()
        }

        fun writeI64(buf: ByteArray, off: Int, v: Long) {
            for (i in 0 until 8) buf[off + i] = ((v shr (8 * i)) and 0xFF).toByte()
        }

        fun readI64(buf: ByteArray, off: Int): Long {
            var v = 0L
            for (i in 7 downTo 0) v = (v shl 8) or (buf[off + i].toLong() and 0xFF)
            return v
        }

        fun writeF32(buf: ByteArray, off: Int, v: Float) {
            writeU32(buf, off, java.lang.Float.floatToIntBits(v))
        }
    }
}
