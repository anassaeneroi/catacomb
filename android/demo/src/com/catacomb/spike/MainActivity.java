package com.catacomb.spike;

import android.app.Activity;
import android.graphics.Color;
import android.graphics.Typeface;
import android.os.Bundle;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/**
 * Minimal demo that exercises the Rust JNI core on-device (Stage-1, Phase 4).
 *
 * <p>The UI is built programmatically (no layout XML / AndroidX / Compose) so
 * the APK compiles against {@code android.jar} alone and stays dependency-free.
 * It lets you type a URL / paste a yt-dlp error and see the results the shared
 * Rust modules ({@code platform}, {@code error_class}, {@code vtt}) return
 * through the JNI bridge.
 */
public class MainActivity extends Activity {

    private TextView output;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        int pad = dp(16);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(pad, pad, pad, pad);

        TextView title = new TextView(this);
        title.setText("Catacomb — Rust core (JNI) demo");
        title.setTextSize(20);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        title.setPadding(0, 0, 0, dp(12));
        root.addView(title);

        // ── Platform detection ─────────────────────────────────────────────
        final EditText urlField = new EditText(this);
        urlField.setHint("Media URL");
        urlField.setText("https://youtu.be/dQw4w9WgXcQ");
        urlField.setInputType(InputType.TYPE_TEXT_VARIATION_URI);
        root.addView(urlField);

        Button detectBtn = new Button(this);
        detectBtn.setText("Detect platform");
        detectBtn.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                String url = urlField.getText().toString();
                show("platformFromUrl:\n" + RustCore.platformFromUrl(url)
                        + "\n\nplatformDirName: " + RustCore.platformDirName(url));
            }
        });
        root.addView(detectBtn);

        // ── Error classification ───────────────────────────────────────────
        final EditText logField = new EditText(this);
        logField.setHint("yt-dlp log line(s)");
        logField.setText("ERROR: HTTP Error 429: Too Many Requests");
        logField.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
        root.addView(logField);

        Button classifyBtn = new Button(this);
        classifyBtn.setText("Classify error");
        classifyBtn.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                show("classifyError:\n" + RustCore.classifyError(logField.getText().toString()));
            }
        });
        root.addView(classifyBtn);

        // ── VTT parse ──────────────────────────────────────────────────────
        Button vttBtn = new Button(this);
        vttBtn.setText("Parse sample VTT");
        vttBtn.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                String sample = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\nHello from Rust\n\n"
                        + "00:00:04.500 --> 00:00:06.000\nSecond cue\n";
                show("vttParse:\n" + RustCore.vttParse(sample));
            }
        });
        root.addView(vttBtn);

        // ── Output ─────────────────────────────────────────────────────────
        output = new TextView(this);
        output.setPadding(0, dp(16), 0, 0);
        output.setTextIsSelectable(true);
        output.setTypeface(Typeface.MONOSPACE);
        output.setText(loadBanner());

        ScrollView scroll = new ScrollView(this);
        scroll.addView(output);
        root.addView(scroll);

        setContentView(root);
    }

    /** Confirm the .so actually loaded by calling into it once at startup. */
    private String loadBanner() {
        try {
            String probe = RustCore.platformDirName("https://youtu.be/x");
            return "libcatacomb_core.so loaded ✓ (probe → \"" + probe + "\")\n"
                    + "Tap a button to call the Rust core.";
        } catch (Throwable t) {
            return "Failed to call Rust core: " + t;
        }
    }

    private void show(String s) {
        output.setText(s);
    }

    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }
}
