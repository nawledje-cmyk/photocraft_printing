package ai.storyteller.photocraft.android;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.MediaStore;
import android.util.Base64;
import android.util.Log;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.webkit.ConsoleMessage;
import android.webkit.JavascriptInterface;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.webkit.RenderProcessGoneDetail;
import android.widget.Toast;

import androidx.webkit.WebViewAssetLoader;

import java.io.IOException;
import java.io.OutputStream;
import java.util.HashMap;
import java.util.Map;

/**
 * PhotoCraft's browser build, shown full screen in a WebView.
 *
 * The site is bundled under assets/www and served from the WebView's own https origin by
 * {@link WebViewAssetLoader}, so the WebAssembly, ES modules and localStorage behave as on a web
 * host, offline. File › Open uses Android's document picker; Save and Export, which download a
 * file in a browser, are caught by assets/android-bridge.js and written to Downloads/PhotoCraft.
 */
public class MainActivity extends Activity {
    private static final String TAG = "PhotoCraft";
    private static final String HOST = "appassets.androidplatform.net";
    private static final String START_URL = "https://" + HOST + "/assets/www/index.html";
    private static final int PICK_FILES = 1;

    private static final int MAX_CONSOLE_LINES = 200;

    private WebView webView;
    private ValueCallback<Uri[]> pendingChooser;
    private int consoleLines;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ErrorLog.write("activity starting");
        try {
            start(savedInstanceState);
        } catch (Throwable e) {
            // Show what went wrong instead of closing: the log has the details.
            ErrorLog.write("startup failed", e);
            showError("PhotoCraft couldn't start", e.toString());
            return;
        }
        String crash = ErrorLog.takeLastCrash();
        if (crash != null) showError("PhotoCraft closed unexpectedly last time", crash);
    }

    private void start(Bundle savedInstanceState) {

        final WebViewAssetLoader loader = new WebViewAssetLoader.Builder()
                .setDomain(HOST)
                .addPathHandler("/assets/", new WebViewAssetLoader.AssetsPathHandler(this))
                .build();

        ErrorLog.write("creating WebView");
        webView = new WebView(this);
        webView.setBackgroundColor(Color.rgb(0x26, 0x26, 0x26));
        setContentView(webView);
        try {
            goFullScreen();
        } catch (Throwable e) {
            ErrorLog.write("full screen failed (continuing)", e);
        }
        android.content.pm.PackageInfo wv = WebView.getCurrentWebViewPackage();
        ErrorLog.write("WebView " + (wv != null ? wv.packageName + " " + wv.versionName : "unknown"));

        WebSettings s = webView.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setDatabaseEnabled(true);
        s.setAllowFileAccess(false);
        s.setAllowContentAccess(true);
        s.setMediaPlaybackRequiresUserGesture(false);
        s.setSupportZoom(false);
        s.setBuiltInZoomControls(false);
        s.setDisplayZoomControls(false);
        s.setTextZoom(100);
        s.setSupportMultipleWindows(false);

        webView.addJavascriptInterface(new Bridge(), "PhotoCraftAndroid");

        webView.setWebViewClient(new WebViewClient() {
            @Override
            public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                WebResourceResponse r = loader.shouldInterceptRequest(request.getUrl());
                if (r != null) {
                    String path = request.getUrl().getPath();
                    // Streaming compilation needs this type; some WebView versions don't know .wasm.
                    if (path != null && path.endsWith(".wasm")) r.setMimeType("application/wasm");
                    else if (path != null && (path.endsWith(".js") || path.endsWith(".mjs"))) r.setMimeType("text/javascript");
                }
                return r;
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri url = request.getUrl();
                if (HOST.equals(url.getHost())) return false;
                // Links out of the editor (help, website) open in the browser, not over the editor.
                try {
                    startActivity(new Intent(Intent.ACTION_VIEW, url));
                } catch (ActivityNotFoundException e) {
                    Log.w(TAG, "no app opens " + url);
                }
                return true;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                ErrorLog.write("page loaded: " + url);
            }

            @Override
            public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                ErrorLog.write("load error " + error.getErrorCode() + " " + error.getDescription() + ": " + request.getUrl());
            }

            @Override
            public void onReceivedHttpError(WebView view, WebResourceRequest request, WebResourceResponse response) {
                ErrorLog.write("HTTP " + response.getStatusCode() + ": " + request.getUrl());
            }

            @Override
            public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
                ErrorLog.write("editor page stopped (" + (detail.didCrash() ? "crashed" : "killed for memory")
                        + ", priority " + detail.rendererPriorityAtExit() + ")");
                // The editor's page was stopped, usually for memory. Start it again rather than close.
                Toast.makeText(MainActivity.this,
                        "PhotoCraft ran out of memory and restarted. Unsaved work was lost.",
                        Toast.LENGTH_LONG).show();
                recreate();
                return true;
            }
        });

        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage m) {
                // The editor's own errors and warnings (and the first lines of anything else).
                boolean important = m.messageLevel() == ConsoleMessage.MessageLevel.ERROR
                        || m.messageLevel() == ConsoleMessage.MessageLevel.WARNING;
                if (consoleLines < MAX_CONSOLE_LINES && (important || consoleLines < 40)) {
                    consoleLines++;
                    ErrorLog.write("console " + m.messageLevel() + ": " + m.message()
                            + " (" + m.sourceId() + ":" + m.lineNumber() + ")");
                }
                return true;
            }

            @Override
            public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (pendingChooser != null) pendingChooser.onReceiveValue(null);
                pendingChooser = callback;
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, params.getMode() == FileChooserParams.MODE_OPEN_MULTIPLE);
                try {
                    startActivityForResult(intent, PICK_FILES);
                } catch (ActivityNotFoundException e) {
                    pendingChooser = null;
                    callback.onReceiveValue(null);
                    return false;
                }
                return true;
            }
        });

        ErrorLog.write("loading " + START_URL);
        if (savedInstanceState != null) webView.restoreState(savedInstanceState);
        else webView.loadUrl(START_URL);
    }

    private void showError(String title, String details) {
        new AlertDialog.Builder(this)
                .setTitle(title)
                .setMessage(details + "\n\nThe full log is in Downloads/PhotoCraft/" + ErrorLog.NAME + ".")
                .setPositiveButton("OK", null)
                .show();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != PICK_FILES) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        ValueCallback<Uri[]> callback = pendingChooser;
        pendingChooser = null;
        if (callback == null) return;
        Uri[] picked = null;
        if (resultCode == RESULT_OK && data != null) {
            if (data.getClipData() != null) {
                int n = data.getClipData().getItemCount();
                picked = new Uri[n];
                for (int i = 0; i < n; i++) picked[i] = data.getClipData().getItemAt(i).getUri();
            } else if (data.getData() != null) {
                picked = new Uri[] {data.getData()};
            }
        }
        callback.onReceiveValue(picked);
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        if (webView != null) webView.saveState(outState);
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus && webView != null) {
            try {
                goFullScreen();
            } catch (Throwable e) {
                ErrorLog.write("full screen failed", e);
            }
        }
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        // Back would close the editor and lose the open documents: ask first.
        new AlertDialog.Builder(this)
                .setTitle("Leave PhotoCraft?")
                .setMessage("Anything you haven't saved will be lost.")
                .setPositiveButton("Leave", (d, w) -> finish())
                .setNegativeButton("Stay", null)
                .show();
    }

    @Override
    protected void onDestroy() {
        if (webView != null) {
            webView.destroy();
            webView = null;
        }
        super.onDestroy();
    }

    @SuppressWarnings("deprecation")
    private void goFullScreen() {
        Window w = getWindow();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            w.setDecorFitsSystemWindows(false);
            WindowInsetsController c = w.getInsetsController();
            if (c != null) {
                c.hide(WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            w.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
        }
    }

    private void toast(String text) {
        runOnUiThread(() -> Toast.makeText(this, text, Toast.LENGTH_LONG).show());
    }

    /** Called from assets/android-bridge.js: a download, sent over in base64 chunks. */
    private final class Bridge {
        private final Map<Integer, Saving> open = new HashMap<>();
        private int next = 1;

        @JavascriptInterface
        public synchronized int begin(String name, String mime) {
            String safe = name.replaceAll("[\\\\/:*?\"<>|\\p{Cntrl}]", "_");
            if (safe.isEmpty()) safe = "photocraft";
            ContentValues v = new ContentValues();
            v.put(MediaStore.MediaColumns.DISPLAY_NAME, safe);
            if (mime != null && !mime.isEmpty()) v.put(MediaStore.MediaColumns.MIME_TYPE, mime);
            v.put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/PhotoCraft");
            v.put(MediaStore.MediaColumns.IS_PENDING, 1);
            ContentResolver cr = getContentResolver();
            try {
                Uri uri = cr.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, v);
                if (uri == null) throw new IOException("Downloads refused the file");
                OutputStream out = cr.openOutputStream(uri);
                if (out == null) throw new IOException("can't write the file");
                int id = next++;
                open.put(id, new Saving(uri, out, safe));
                return id;
            } catch (Exception e) {
                ErrorLog.write("save " + safe + " failed", e);
                toast("Couldn't save " + safe + ": " + e.getMessage());
                return -1;
            }
        }

        @JavascriptInterface
        public synchronized boolean append(int id, String base64) {
            Saving s = open.get(id);
            if (s == null) return false;
            try {
                s.out.write(Base64.decode(base64, Base64.DEFAULT));
                return true;
            } catch (Exception e) {
                fail(id, e.getMessage());
                return false;
            }
        }

        @JavascriptInterface
        public synchronized void finish(int id) {
            Saving s = open.remove(id);
            if (s == null) return;
            try {
                s.out.close();
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.IS_PENDING, 0);
                getContentResolver().update(s.uri, v, null, null);
                toast("Saved to Downloads/PhotoCraft/" + s.name);
            } catch (Exception e) {
                Log.e(TAG, "save " + s.name, e);
                getContentResolver().delete(s.uri, null, null);
                toast("Couldn't save " + s.name + ": " + e.getMessage());
            }
        }

        @JavascriptInterface
        public synchronized void fail(int id, String why) {
            Saving s = open.remove(id);
            String name = s != null ? s.name : "the file";
            if (s != null) {
                try { s.out.close(); } catch (IOException ignored) { }
                getContentResolver().delete(s.uri, null, null);
            }
            toast("Couldn't save " + name + (why != null ? ": " + why : ""));
        }
    }

    private static final class Saving {
        final Uri uri;
        final OutputStream out;
        final String name;

        Saving(Uri uri, OutputStream out, String name) {
            this.uri = uri;
            this.out = out;
            this.name = name;
        }
    }
}
