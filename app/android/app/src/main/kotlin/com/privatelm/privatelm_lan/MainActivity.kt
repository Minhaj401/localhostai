package com.privatelm.privatelm_lan

import android.content.Context
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.net.InetSocketAddress
import java.net.Socket

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "privatelm/native")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    // Bundled rpc-server lives here (extracted, executable).
                    "nativeLibDir" -> result.success(applicationInfo.nativeLibraryDir)
                    // LAN IP for showing the worker address.
                    "localIp" -> {
                        try {
                            val s = Socket()
                            s.connect(InetSocketAddress("8.8.8.8", 80), 1000)
                            val ip = s.localAddress.hostAddress
                            s.close()
                            result.success(ip)
                        } catch (e: Exception) {
                            result.success(null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }
}
