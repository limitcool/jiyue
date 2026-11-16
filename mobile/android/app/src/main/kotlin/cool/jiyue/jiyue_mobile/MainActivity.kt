package cool.jiyue.jiyue_mobile

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.Settings
import androidx.core.content.FileProvider
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.File

class MainActivity : FlutterActivity() {
    companion object {
        private const val UPDATER_CHANNEL = "cool.jiyue.jiyue_mobile/app_updater"
        private const val PLATFORM_CHANNEL = "cool.jiyue.jiyue_mobile/platform"
        private const val REQ_MANAGE_STORAGE = 4101
        private const val REQ_LEGACY_STORAGE = 4102
        private const val REQ_PICK_DIRECTORY = 4103
        private const val REQ_PICK_IMAGE = 4104
    }

    private var pendingStorageResult: MethodChannel.Result? = null
    private var pendingDirectoryResult: MethodChannel.Result? = null
    private var pendingImageResult: MethodChannel.Result? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, UPDATER_CHANNEL)
            .setMethodCallHandler { call, result ->
                if (call.method == "getAppVersion") {
                    result.success(readAppVersion())
                } else if (!FlavorUpdater.handle(this, call, result)) {
                    result.notImplemented()
                }
            }

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, PLATFORM_CHANNEL)
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "hasStoragePermission" -> result.success(hasStoragePermission())
                    "requestStoragePermission" -> handleRequestStoragePermission(result)
                    "pickDirectory" -> handlePickDirectory(result)
                    "pickImageBytes" -> handlePickImageBytes(result)
                    "shareFile" -> {
                        val filePath = call.argument<String>("filePath")
                        val title = call.argument<String>("title") ?: "导出笔记"
                        if (filePath.isNullOrBlank()) {
                            result.error("INVALID_ARGS", "Missing filePath", null)
                        } else {
                            handleShareFile(filePath, title, result)
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
            info.versionName ?: "0.0.0"
        } catch (_: Exception) {
            "0.0.0"
        }
    }

    private fun hasStoragePermission(): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            return Environment.isExternalStorageManager()
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            val readOk = checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
            val writeOk = checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
            return readOk && writeOk
        }
        return true
    }

    private fun handleRequestStoragePermission(result: MethodChannel.Result) {
        if (hasStoragePermission()) {
            result.success(true)
            return
        }
        pendingStorageResult?.success(false)
        pendingStorageResult = result

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            try {
                val intent = Intent(
                    Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                    Uri.parse("package:$packageName")
                )
                @Suppress("DEPRECATION")
                startActivityForResult(intent, REQ_MANAGE_STORAGE)
            } catch (_: Exception) {
                try {
                    val intent = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                    @Suppress("DEPRECATION")
                    startActivityForResult(intent, REQ_MANAGE_STORAGE)
                } catch (e: Exception) {
                    pendingStorageResult = null
                    result.error("PERMISSION_FAILED", e.message, null)
                }
            }
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            requestPermissions(
                arrayOf(
                    Manifest.permission.READ_EXTERNAL_STORAGE,
                    Manifest.permission.WRITE_EXTERNAL_STORAGE
                ),
                REQ_LEGACY_STORAGE
            )
        } else {
            pendingStorageResult = null
            result.success(true)
        }
    }

    private fun handlePickDirectory(result: MethodChannel.Result) {
        pendingDirectoryResult?.success(null)
        pendingDirectoryResult = result
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                addFlags(Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
            }
            @Suppress("DEPRECATION")
            startActivityForResult(intent, REQ_PICK_DIRECTORY)
        } catch (e: Exception) {
            pendingDirectoryResult = null
            result.error("PICK_DIR_FAILED", e.message ?: "无法打开目录选择器", null)
        }
    }

    private fun handlePickImageBytes(result: MethodChannel.Result) {
        pendingImageResult?.success(null)
        pendingImageResult = result
        try {
            val intent = Intent(Intent.ACTION_GET_CONTENT).apply {
                type = "image/*"
                addCategory(Intent.CATEGORY_OPENABLE)
            }
            @Suppress("DEPRECATION")
            startActivityForResult(Intent.createChooser(intent, "选择二维码图片"), REQ_PICK_IMAGE)
        } catch (e: Exception) {
            pendingImageResult = null
            result.error("PICK_IMAGE_FAILED", e.message ?: "无法打开图片选择器", null)
        }
    }

    private fun handleShareFile(
        filePath: String,
        title: String,
        result: MethodChannel.Result,
    ) {
        val file = File(filePath)
        if (!file.exists() || !file.isFile) {
            result.error("FILE_NOT_FOUND", "文件不存在: $filePath", null)
            return
        }

        try {
            val authority = "${applicationContext.packageName}.fileprovider"
            val uri = try {
                FileProvider.getUriForFile(this, authority, file)
            } catch (_: IllegalArgumentException) {
                // External SD cards or private dirs outside configured roots fall back to cache/exports.
                val exportDir = File(cacheDir, "exports")
                if (!exportDir.exists()) exportDir.mkdirs()
                val staged = File(exportDir, file.name)
                file.copyTo(staged, overwrite = true)
                FileProvider.getUriForFile(this, authority, staged)
            }

            val intent = Intent(Intent.ACTION_SEND).apply {
                type = "text/markdown"
                putExtra(Intent.EXTRA_STREAM, uri)
                putExtra(Intent.EXTRA_SUBJECT, title)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(Intent.createChooser(intent, title))
            result.success(true)
        } catch (e: Exception) {
            result.error("SHARE_FAILED", e.message ?: "分享导出失败", null)
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_LEGACY_STORAGE) {
            val pending = pendingStorageResult
            pendingStorageResult = null
            pending?.success(hasStoragePermission())
        }
    }

    @Deprecated("Required for FlutterActivity startActivityForResult compatibility")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        when (requestCode) {
            REQ_MANAGE_STORAGE -> {
                val pending = pendingStorageResult
                pendingStorageResult = null
                pending?.success(hasStoragePermission())
            }
            REQ_PICK_DIRECTORY -> {
                val pending = pendingDirectoryResult
                pendingDirectoryResult = null
                if (pending == null) return
                if (resultCode != Activity.RESULT_OK || data?.data == null) {
                    pending.success(null)
                    return
                }
                val treeUri = data.data!!
                try {
                    contentResolver.takePersistableUriPermission(
                        treeUri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    )
                } catch (_: Exception) {
                    // Best-effort persistable grant; direct filesystem access relies on storage permission.
                }
                val resolved = resolveTreeUriToPath(treeUri)
                if (resolved != null) {
                    pending.success(resolved)
                } else {
                    pending.error(
                        "UNSUPPORTED_TREE_URI",
                        "无法从系统选择器解析本地路径，请确认选择的是本机内部存储目录",
                        null
                    )
                }
            }
            REQ_PICK_IMAGE -> {
                val pending = pendingImageResult
                pendingImageResult = null
                if (pending == null) return
                if (resultCode != Activity.RESULT_OK || data?.data == null) {
                    pending.success(null)
                    return
                }
                try {
                    val bytes = contentResolver.openInputStream(data.data!!)?.use { it.readBytes() }
                    pending.success(bytes)
                } catch (e: Exception) {
                    pending.error("READ_IMAGE_FAILED", e.message ?: "读取图片失败", null)
                }
            }
        }
    }

    private fun resolveTreeUriToPath(treeUri: Uri): String? {
        val docId = DocumentsContract.getTreeDocumentId(treeUri) ?: return null
        if (docId.startsWith("raw:")) {
            return docId.removePrefix("raw:")
        }
        val parts = docId.split(":", limit = 2)
        val volume = parts[0]
        val rel = if (parts.size > 1) parts[1].trimStart('/') else ""

        if (volume.equals("primary", ignoreCase = true)) {
            val base = Environment.getExternalStorageDirectory().absolutePath
            return if (rel.isEmpty()) base else "$base/$rel"
        }
        if (volume.equals("home", ignoreCase = true)) {
            val base = Environment.getExternalStorageDirectory().absolutePath
            return if (rel.isEmpty()) "$base/Documents" else "$base/Documents/$rel"
        }
        if (volume.isNotBlank()) {
            val candidate = if (rel.isEmpty()) "/storage/$volume" else "/storage/$volume/$rel"
            if (File(candidate).exists()) {
                return candidate
            }
        }
        return null
    }
}
