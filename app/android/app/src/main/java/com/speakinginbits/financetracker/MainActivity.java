package com.speakinginbits.financetracker;

import android.app.NativeActivity;
import android.os.Build;
import android.view.View;
import android.view.WindowInsetsController;

/**
 * Android's NativeActivity (which loads the Rust app), plus a hook the Rust side calls so the
 * status and navigation bar icons match the app's theme rather than only the system's.
 */
public class MainActivity extends NativeActivity {
    /**
     * Dark bar icons when {@code light} is true (for the light theme), light icons otherwise.
     * Called from the Rust thread, so the change is posted to the UI thread.
     */
    public void setLightSystemBars(final boolean light) {
        runOnUiThread(() -> {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsetsController controller = getWindow().getInsetsController();
                if (controller == null) return;
                int mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                        | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                controller.setSystemBarsAppearance(light ? mask : 0, mask);
            } else {
                View decor = getWindow().getDecorView();
                int flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                int visibility = decor.getSystemUiVisibility();
                decor.setSystemUiVisibility(light ? visibility | flags : visibility & ~flags);
            }
        });
    }
}
