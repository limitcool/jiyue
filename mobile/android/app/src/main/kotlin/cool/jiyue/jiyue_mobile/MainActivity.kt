package cool.jiyue.jiyue_mobile

import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File

class MainActivity : FlutterActivity() {
    companion object {
        private const val UPDATER_CHANNEL = "cool.jiyue.jiyue_mobile/app_updater"
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, UPDATER_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "getAppVersion" -> result.success(readAppVersion())
                    "getUpdatesDir" -> {
                        val dir = File(cacheDir, "updates")
                        if (!dir.exists()) {
                            dir.mkdirs()
                        }
                        result.success(dir.absolutePath)
                    }
                    "canInstallPackages" -> result.success(canInstallPackages())
                    "openInstallPermissionSettings" -> {
                        openInstallPermissionSettings()
                        result.success(true)
                    }
                    "installApk" -> {
                        val filePath = call.argument<String>("filePath")
                        if (filePath.isNullOrBlank()) {
                            result.error("INVALID_ARGS", "Missing filePath", null)
                        } else {
                            handleInstallApk(filePath, result)
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }

    private fun readAppVersion(): String {
        return try {
            val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                packageManager.getPackageInfo(
                    packageName,
                    PackageManager.PackageInfoFlags.of(0)
                )
            } else {
                @Suppress("DEPRECATION")
                packageManager.getPackageInfo(packageName, 0)
            }
            info.versionName ?: "0.3.5"
        } catch (_: Exception) {
            "0.3.5"
        }
    }

    private fun canInstallPackages(): Boolean {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            packageManager.canRequestPackageInstalls()
        } else {
            true
        }
    }

    private fun openInstallPermissionSettings() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val intent = Intent(
                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:$packageName")
            ).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            startActivity(intent)
        }
    }

    private fun handleInstallApk(filePath: String, result: MethodChannel.Result) {
        val file = File(filePath)
        if (!file.exists() || !file.isFile) {
            result.error("FILE_NOT_FOUND", "APK 文件不存在: $filePath", null)
            return
        }

        if (!canInstallPackages()) {
            openInstallPermissionSettings()
            result.success("PERMISSION_REQUIRED")
            return
        }

        try {
            val authority = "${applicationContext.packageName}.fileprovider"
            val contentUri = FileProvider.getUriForFile(this, authority, file)
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(contentUri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(intent)
            result.success("LAUNCHED")
        } catch (e: Exception) {
            result.error("INSTALL_FAILED", e.message ?: "无法拉起系统安装器", null)
        }
    }
}
