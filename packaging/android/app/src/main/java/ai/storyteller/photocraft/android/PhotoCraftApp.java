package ai.storyteller.photocraft.android;

import android.app.Application;

/** Starts the error log before anything else, so a crash during startup is recorded too. */
public class PhotoCraftApp extends Application {
    @Override
    public void onCreate() {
        super.onCreate();
        ErrorLog.init(this);
    }
}
