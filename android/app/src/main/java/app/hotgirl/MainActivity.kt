package app.hotgirl

import android.Manifest
import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : AppCompatActivity() {

    private lateinit var prefs: SharedPreferences
    private lateinit var status: TextView
    private val handler = Handler(Looper.getMainLooper())

    private val pollStatus = object : Runnable {
        override fun run() {
            if (HeadTrackerService.running.get()) {
                val err = HeadTrackerService.lastError.get()
                status.text = getString(
                    R.string.status_diag,
                    HeadTrackerService.samplesSent.get(),
                    HeadTrackerService.sendErrors.get(),
                    err ?: "—"
                )
            }
            handler.postDelayed(this, 500)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        prefs = getSharedPreferences("hotgirl", Context.MODE_PRIVATE)
        val hostEdit = findViewById<EditText>(R.id.host)
        val portEdit = findViewById<EditText>(R.id.port)
        status = findViewById(R.id.status)
        val start = findViewById<Button>(R.id.start)
        val stop = findViewById<Button>(R.id.stop)

        hostEdit.setText(prefs.getString("host", ""))
        portEdit.setText(prefs.getInt("port", HeadTrackerService.DEFAULT_PORT).toString())

        start.setOnClickListener {
            val host = hostEdit.text.toString().trim()
            val port = portEdit.text.toString().toIntOrNull() ?: HeadTrackerService.DEFAULT_PORT
            if (host.isEmpty()) {
                status.text = getString(R.string.status_missing_host)
                return@setOnClickListener
            }
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                ActivityCompat.requestPermissions(
                    this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1
                )
            }
            prefs.edit().putString("host", host).putInt("port", port).apply()
            try {
                HeadTrackerService.start(this, host, port)
                status.text = getString(R.string.status_running)
            } catch (e: Exception) {
                status.text = getString(R.string.status_error, e)
                Toast.makeText(this, getString(R.string.status_error, e), Toast.LENGTH_LONG).show()
            }
        }

        stop.setOnClickListener {
            HeadTrackerService.stop(this)
            status.text = getString(R.string.status_stopped)
        }
    }

    override fun onResume() {
        super.onResume()
        handler.post(pollStatus)
    }

    override fun onPause() {
        super.onPause()
        handler.removeCallbacks(pollStatus)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 1 && grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            Toast.makeText(this, R.string.permission_granted, Toast.LENGTH_SHORT).show()
        }
    }
}
