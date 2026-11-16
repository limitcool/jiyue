package cool.jiyue.jiyue_mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.File

object FlavorUpdater {
    fun handle(activity: Activity, call: MethodCall, result: MethodChannel.Result): Boolean {
        return when (call.method) {
            "getUpdatesDir" -> {
                val dir = File(activity.cacheDir, "updates")
                if (!dir.exists()) {
                    dir.mkdirs()
                }
                result.success(dir.absolutePath)
                true
            }
            "canInstallPackages" -> {
                result.success(canInstallPackages(activity))
                true
            }
            "openInstallPermissionSettings" -> {
                openInstallPermissionSettings(activity)
                result.success(true)
                true
            }
            "installApk" -> {
                val filePath = call.argument<String>("filePath")
                if (filePath.isNullOrBlank()) {
                    result.error("INVALID_ARGS", "Missing filePath", null)
                } else {
                    handleInstallApk(activity, filePath, result)
                }
                true
            }
            else -> false
        }
    }

    private fun canInstallPackages(activity: Activity): Boolean {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            activity.packageManager.canRequestPackageInstalls()
        } else {
            true
        }
    }

    private fun openInstallPermissionSettings(activity: Activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val intent = Intent(
                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:${activity.packageName}")
            ).apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            activity.startActivity(intent)
        }
    }

    private fun handleInstallApk(
        activity: Activity,
        filePath: String,
        result: MethodChannel.Result,
    ) {
        val file = File(filePath)
        if (!file.exists() || !file.isFile) {
            result.error("FILE_NOT_FOUND", "APK 文件不存在: $filePath", null)
            return
        }

        if (!canInstallPackages(activity)) {
            openInstallPermissionSettings(activity)
            result.success("PERMISSION_REQUIRED")
            return
        }

        try {
            val authority = "${activity.applicationContext.packageName}.fileprovider"
            val contentUri = FileProvider.getUriForFile(activity, authority, file)
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(contentUri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.startActivity(intent)
            result.success("LAUNCHED")
        } catch (e: Exception) {
            result.error("INSTALL_FAILED", e.message ?: "无法拉起系统安装器", null)
        }
    }
}
