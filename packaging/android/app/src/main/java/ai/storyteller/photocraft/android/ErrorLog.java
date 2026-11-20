package ai.storyteller.photocraft.android;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;

/**
 * The app's error log: startup steps, crashes, and errors from the editor's page.
 *
 * Kept in the app's own storage and copied to Downloads/PhotoCraft/PhotoCraft-error-log.txt, where
 * the person can open or share it with any file manager. A crash is also kept on its own so the next
 * start can show it.
 */
final class ErrorLog {
    static final String NAME = "PhotoCraft-error-log.txt";
    private static final String TAG = "PhotoCraft";
    private static final long MAX_BYTES = 256 * 1024;
    private static final String FOLDER = Environment.DIRECTORY_DOWNLOADS + "/PhotoCraft/";

    private static Context app;
    private static File file;
    private static File lastCrash;
    private static File running;
    private static String unexpectedEnd;

    private ErrorLog() { }

    static synchronized void init(Context context) {
        app = context.getApplicationContext();
        file = new File(app.getFilesDir(), NAME);
        lastCrash = new File(app.getFilesDir(), "last-crash.txt");
        // Keep the log small: start over once it is large.
        running = new File(app.getFilesDir(), "running");
        // The marker is there while the app is on screen; still there now means the last run
        // ended without closing, e.g. the WebView's graphics stopped the whole app.
        if (running.exists()) {
            unexpectedEnd = tail(12);
            running.delete();
        }
        if (file.length() > MAX_BYTES) file.delete();

        final Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, error) -> {
            try {
                String trace = stackTrace(error);
                write("CRASH on thread " + thread.getName() + "\n" + trace);
                if (running != null) running.delete();
                try (FileOutputStream out = new FileOutputStream(lastCrash)) {
                    out.write((stamp() + "\n" + trace).getBytes(StandardCharsets.UTF_8));
                }
            } catch (Throwable ignored) {
                // Never let logging hide the original crash.
            }
            if (previous != null) previous.uncaughtException(thread, error);
        });

        write("---- PhotoCraft " + version() + " starting on Android " + Build.VERSION.RELEASE
                + " (API " + Build.VERSION.SDK_INT + "), " + Build.MANUFACTURER + " " + Build.MODEL
                + ", " + Build.SUPPORTED_ABIS[0]);
    }

    /** Adds a line to the log and refreshes the copy in Downloads. */
    static synchronized void write(String message) {
        Log.i(TAG, message);
        if (file == null) return;
        try (FileOutputStream out = new FileOutputStream(file, true)) {
            out.write((stamp() + "  " + message + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            Log.w(TAG, "error log", e);
            return;
        }
        publish();
    }

    static void write(String message, Throwable error) {
        write(message + "\n" + stackTrace(error));
    }

    /** The last lines of the log when the last run ended while on screen, else null. */
    static synchronized String lastRunEndedUnexpectedly() {
        return unexpectedEnd;
    }

    static synchronized void markRunning(boolean on) {
        if (running == null) return;
        try {
            if (on) {
                if (!running.exists() && !running.createNewFile()) Log.w(TAG, "no run marker");
            } else {
                running.delete();
            }
        } catch (IOException e) {
            Log.w(TAG, "run marker", e);
        }
    }

    private static String tail(int lines) {
        try {
            java.util.List<String> all = Files.readAllLines(file.toPath(), StandardCharsets.UTF_8);
            return String.join("\n", all.subList(Math.max(0, all.size() - lines), all.size()));
        } catch (Exception e) {
            return "(no log)";
        }
    }

    /** The crash from the last run, once; null when the last run didn't crash. */
    static synchronized String takeLastCrash() {
        if (lastCrash == null || !lastCrash.exists()) return null;
        try {
            return new String(Files.readAllBytes(lastCrash.toPath()), StandardCharsets.UTF_8);
        } catch (IOException e) {
            return null;
        } finally {
            lastCrash.delete();
        }
    }

    /** Copies the log to Downloads/PhotoCraft, replacing the previous copy. */
    private static void publish() {
        try {
            ContentResolver cr = app.getContentResolver();
            Uri collection = MediaStore.Downloads.EXTERNAL_CONTENT_URI;
            Uri uri = null;
            try (Cursor c = cr.query(collection, new String[] {MediaStore.MediaColumns._ID},
                    MediaStore.MediaColumns.DISPLAY_NAME + "=? AND " + MediaStore.MediaColumns.RELATIVE_PATH + "=?",
                    new String[] {NAME, FOLDER}, null)) {
                if (c != null && c.moveToFirst()) uri = Uri.withAppendedPath(collection, Long.toString(c.getLong(0)));
            }
            if (uri == null) {
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.DISPLAY_NAME, NAME);
                v.put(MediaStore.MediaColumns.MIME_TYPE, "text/plain");
                v.put(MediaStore.MediaColumns.RELATIVE_PATH, FOLDER);
                uri = cr.insert(collection, v);
                if (uri == null) return;
            }
            try (OutputStream out = cr.openOutputStream(uri, "wt")) {
                if (out != null) out.write(Files.readAllBytes(file.toPath()));
            }
        } catch (Throwable e) {
            // The copy is a convenience; the log in app storage is still written.
            Log.w(TAG, "error log copy", e);
        }
    }

    private static String version() {
        try {
            return app.getPackageManager().getPackageInfo(app.getPackageName(), 0).versionName;
        } catch (Exception e) {
            return "?";
        }
    }

    private static String stamp() {
        return new SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(new Date());
    }

    static String stackTrace(Throwable error) {
        StringWriter s = new StringWriter();
        error.printStackTrace(new PrintWriter(s));
        return s.toString();
    }
}
