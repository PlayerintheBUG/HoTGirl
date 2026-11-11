package app.hotgirl

import android.content.Context
import android.content.SharedPreferences
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity

class MainActivity : AppCompatActivity() {

    private lateinit var prefs: SharedPreferences

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        prefs = getSharedPreferences("hotgirl", Context.MODE_PRIVATE)
        val hostEdit = findViewById<EditText>(R.id.host)
        val portEdit = findViewById<EditText>(R.id.port)
        val status = findViewById<TextView>(R.id.status)
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
            prefs.edit().putString("host", host).putInt("port", port).apply()
            HeadTrackerService.start(this, host, port)
            status.text = getString(R.string.status_running)
        }

        stop.setOnClickListener {
            HeadTrackerService.stop(this)
            status.text = getString(R.string.status_stopped)
        }
    }
}
