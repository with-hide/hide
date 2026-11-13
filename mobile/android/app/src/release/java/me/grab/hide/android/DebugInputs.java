package me.grab.hide.android;

import android.content.Intent;
import com.getcapacitor.Bridge;

/** A release build takes no input from outside the page: src/debug holds the emulator check's. */
final class DebugInputs {

    private DebugInputs() {}

    static void attach(ShellActivity activity, Bridge bridge, Intent intent) {}
}
