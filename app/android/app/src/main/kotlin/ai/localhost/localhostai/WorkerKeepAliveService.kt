package ai.localhost.localhostai

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import java.io.File

/** Keeps user-requested LAN compute alive while the screen is off. */
class WorkerKeepAliveService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var workerPid: Int? = null
    private var ready = false
    private var stopNotified = false
    private val cleanup = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    // The foreground notification bounds this lock to explicit sharing;
    // onDestroy releases it on every stop/failure path.
    @SuppressLint("WakelockTimeout")
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            requestStopAndFinish()
            return START_NOT_STICKY
        }
        try {
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (Build.VERSION.SDK_INT >= 26) {
                manager.createNotificationChannel(NotificationChannel(
                    CHANNEL, "Phone compute sharing", NotificationManager.IMPORTANCE_LOW
                ))
            }
            val open = PendingIntent.getActivity(this, 0,
                Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val stop = PendingIntent.getService(this, 1,
                Intent(this, WorkerKeepAliveService::class.java).setAction(ACTION_STOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val builder = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL) else Notification.Builder(this)
            val notification = builder
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentTitle("localhost.ai is sharing compute")
                .setContentText("Phone worker is available to your paired laptop.")
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .addAction(Notification.Action.Builder(null, "Stop sharing", stop).build())
                .build()
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            if (!ready) {
                val power = getSystemService(Context.POWER_SERVICE) as PowerManager
                wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "localhost.ai:rpc").apply {
                    setReferenceCounted(false)
                    acquire()
                }
                val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                @Suppress("DEPRECATION")
                val lock = wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "localhost.ai:rpc")
                wifiLock = lock.apply { setReferenceCounted(false); acquire() }
                ready = true
            }
            finishStarts(null)
        } catch (error: Exception) {
            finishStarts(error)
            stopSelf()
        }
        // A system restart must not silently re-enable sharing without Dart.
        return START_NOT_STICKY
    }

    private fun requestStop() {
        if (!stopNotified) {
            stopNotified = true
            onStop?.invoke()
        }
    }

    private fun requestStopAndFinish() {
        requestStop()
        // Let Dart withdraw the session and stop the child first. If its engine
        // is gone or unresponsive, service destruction performs final cleanup.
        if (onStop == null) stopSelf()
        else cleanup.postDelayed({ stopSelf() }, 5000)
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        requestStopAndFinish()
    }

    override fun onDestroy() {
        cleanup.removeCallbacksAndMessages(null)
        ready = false
        requestStop()
        // Fallback if the Flutter engine has already detached. Never signal a
        // reused PID belonging to anything other than our bundled executable.
        workerPid?.let { pid ->
            try {
                val executable = File("/proc/$pid/cmdline").readText().substringBefore('\u0000')
                if (executable == "${applicationInfo.nativeLibraryDir}/libggmlrpc.so") {
                    android.os.Process.killProcess(pid)
                }
            } catch (_: Exception) { }
        }
        workerPid = null
        if (wifiLock?.isHeld == true) wifiLock?.release()
        if (wakeLock?.isHeld == true) wakeLock?.release()
        wifiLock = null
        wakeLock = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        instance = null
        finishStarts(IllegalStateException("Phone sharing service stopped"))
        val callbacks = stops.toList()
        stops.clear()
        callbacks.forEach { it() }
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "localhost_compute"
        private const val NOTIFICATION_ID = 50052
        private const val ACTION_STOP = "com.localhostai.STOP_SHARING"
        private var instance: WorkerKeepAliveService? = null
        private val starts = mutableListOf<(Throwable?) -> Unit>()
        private val stops = mutableListOf<() -> Unit>()
        var onStop: (() -> Unit)? = null

        private fun finishStarts(error: Throwable?) {
            val callbacks = starts.toList()
            starts.clear()
            callbacks.forEach { it(error) }
        }

        fun start(context: Context, completed: (Throwable?) -> Unit) {
            if (instance?.ready == true) { completed(null); return }
            starts.add(completed)
            if (starts.size > 1) return
            try {
                val intent = Intent(context, WorkerKeepAliveService::class.java)
                if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent) else context.startService(intent)
            } catch (error: Exception) { finishStarts(error) }
        }

        fun track(pid: Int) {
            check(instance?.ready == true) { "Phone sharing service is not ready" }
            instance?.workerPid = pid
        }

        fun clear(pid: Int) {
            if (instance?.workerPid == pid) instance?.workerPid = null
        }

        fun stop(context: Context, completed: () -> Unit) {
            if (instance == null) {
                context.stopService(Intent(context, WorkerKeepAliveService::class.java))
                finishStarts(IllegalStateException("Phone sharing start cancelled"))
                completed()
                return
            }
            stops.add(completed)
            instance?.ready = false
            instance?.stopNotified = true
            context.stopService(Intent(context, WorkerKeepAliveService::class.java))
        }
    }
}
