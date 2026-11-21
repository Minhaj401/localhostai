package ai.localhost.localhostai

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.BatteryManager
import android.os.PowerManager
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.net.Inet4Address
import java.net.NetworkInterface

class MainActivity : FlutterActivity() {
    private var workerStopCallback: (() -> Unit)? = null
    private var exportResult: MethodChannel.Result? = null
    private var exportContent: String? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val channel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "localhostai/native")
        workerStopCallback = { channel.invokeMethod("stopSharing", null) }
        WorkerKeepAliveService.onStop = workerStopCallback
        channel.setMethodCallHandler { call, result ->
                when (call.method) {
                    "exportConversation" -> {
                        val content = call.argument<String>("content")
                        val name = call.argument<String>("filename")
                        if (exportResult != null) {
                            result.error("export_busy", "Finish the current export first", null)
                        } else if (content == null || name == null || content.length > 4 * 1024 * 1024) {
                            result.error("invalid_export", "Invalid conversation export", null)
                        } else {
                            exportResult = result
                            exportContent = content
                            try {
                                startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                                    addCategory(Intent.CATEGORY_OPENABLE)
                                    type = call.argument<String>("mime") ?: "text/plain"
                                    putExtra(Intent.EXTRA_TITLE, name)
                                }, 8201)
                            } catch (error: Exception) {
                                exportResult = null
                                exportContent = null
                                result.error("export_failed", error.message, null)
                            }
                        }
                    }
                    // Bundled rpc-server lives here (extracted, executable).
                    "nativeLibDir" -> result.success(applicationInfo.nativeLibraryDir)
                    "cacheDir" -> result.success(cacheDir.absolutePath)
                    "workerLogDir" -> result.success(filesDir.absolutePath)
                    "powerState" -> {
                        val battery = registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
                        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
                        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
                        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
                        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
                        val powered = charging || (battery?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
                        val thermal = if (Build.VERSION.SDK_INT >= 29) {
                            (getSystemService(Context.POWER_SERVICE) as PowerManager).currentThermalStatus
                        } else PowerManager.THERMAL_STATUS_NONE
                        result.success(mapOf(
                            "battery_percent" to if (level >= 0) level * 100 / scale.coerceAtLeast(1) else -1,
                            "powered" to powered,
                            "thermal_status" to thermal
                        ))
                    }
                    "startWorkerProtection" -> {
                        WorkerKeepAliveService.start(this) { error ->
                            if (error == null) {
                                result.success(null)
                                if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                                    requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 50052)
                                }
                            } else result.error("worker_protection", error.message, null)
                        }
                    }
                    "trackWorker" -> {
                        try {
                            WorkerKeepAliveService.track(call.argument<Int>("pid")!!)
                            result.success(null)
                        } catch (error: Exception) { result.error("worker_protection", error.message, null) }
                    }
                    "clearWorker" -> {
                        call.argument<Int>("pid")?.let { WorkerKeepAliveService.clear(it) }
                        result.success(null)
                    }
                    "stopWorkerProtection" -> WorkerKeepAliveService.stop(this) { result.success(null) }
                    // OEM background limits kill workers mid-load; an explicit
                    // exemption is the strongest keep-alive after the
                    // foreground service.
                    "batteryOptimizationIgnored" -> {
                        val power = getSystemService(Context.POWER_SERVICE) as PowerManager
                        result.success(power.isIgnoringBatteryOptimizations(packageName))
                    }
                    "requestIgnoreBatteryOptimization" -> {
                        try {
                            startActivity(Intent(
                                Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                                Uri.parse("package:$packageName")
                            ))
                            result.success(null)
                        } catch (error: Exception) {
                            result.error("battery_request_failed", error.message, null)
                        }
                    }
                    // LAN IP for showing the worker address.
                    "localIp" -> {
                        try {
                            result.success(lanAddress())
                        } catch (e: Exception) {
                            result.success(null)
                        }
                    }
                    "lanNetwork" -> {
                        val manager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
                        val value = manager.allNetworks.asSequence().mapNotNull { network ->
                            val capabilities = manager.getNetworkCapabilities(network)
                            if (capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true ||
                                (capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) != true &&
                                 capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) != true)) return@mapNotNull null
                            manager.getLinkProperties(network)?.linkAddresses?.firstOrNull {
                                it.address is Inet4Address && !it.address.isLoopbackAddress
                            }
                        }.firstOrNull()
                        result.success(value?.let { mapOf("address" to it.address.hostAddress, "prefix" to it.prefixLength) })
                    }
                    else -> result.notImplemented()
                }
            }
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        exportResult?.error("export_cancelled", "App closed during export", null)
        exportResult = null
        exportContent = null
        if (WorkerKeepAliveService.onStop === workerStopCallback) {
            WorkerKeepAliveService.onStop = null
            WorkerKeepAliveService.stop(this) { }
        }
        workerStopCallback = null
        super.cleanUpFlutterEngine(flutterEngine)
    }

    private fun lanAddress(): String? {
        val manager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        // Prefer LAN interfaces even when Android routes Internet traffic over
        // cellular. No Internet connection or main-thread socket is needed.
        val networks = manager.allNetworks.filter { network ->
            val capabilities = manager.getNetworkCapabilities(network)
            capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) != true &&
                (capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true ||
                 capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true)
        }
        for (network in networks) {
            val address = manager.getLinkProperties(network)?.linkAddresses
                ?.map { it.address }
                ?.firstOrNull { it is Inet4Address && !it.isLoopbackAddress }
            if (address != null) return address.hostAddress
        }
        return NetworkInterface.getNetworkInterfaces()?.toList()
            ?.filter { it.isUp && !it.isLoopback && !it.isPointToPoint &&
                (it.name.startsWith("wlan") || it.name.startsWith("swlan") ||
                 it.name.startsWith("ap") || it.name.startsWith("eth") || it.name.startsWith("en")) }
            ?.flatMap { it.inetAddresses.toList() }
            ?.firstOrNull { it is Inet4Address && it.isSiteLocalAddress }
            ?.hostAddress
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != 8201) return
        val result = exportResult ?: return
        val content = exportContent
        exportResult = null
        exportContent = null
        val uri = data?.data
        if (resultCode != RESULT_OK || uri == null || content == null) {
            result.success(false)
            return
        }
        Thread {
            try {
                val output = contentResolver.openOutputStream(uri, "wt")
                    ?: throw java.io.IOException("Cannot open selected document")
                output.use { it.write(content.toByteArray(Charsets.UTF_8)) }
                runOnUiThread { result.success(true) }
            } catch (error: Exception) {
                runOnUiThread { result.error("export_failed", error.message, null) }
            }
        }.start()
    }
}
