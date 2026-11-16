package cool.jiyue.jiyue_mobile

import android.app.Activity
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/** FOSS flavor stub: in-app self-update installation is intentionally omitted for F-Droid compliance. */
object FlavorUpdater {
    fun handle(activity: Activity, call: MethodCall, result: MethodChannel.Result): Boolean {
        return false
    }
}
