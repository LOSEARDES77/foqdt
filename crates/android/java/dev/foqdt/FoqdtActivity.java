package dev.foqdt;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Color;
import android.graphics.SurfaceTexture;
import android.hardware.Camera;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.webkit.MimeTypeMap;
import android.widget.Toast;
import java.io.File;
import java.io.FileOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/**
 * NativeActivity plus the few things Rust cannot do without Java: camera frames,
 * the runtime camera permission and the document picker.
 * Every method called from Rust returns a byte[] (or null) to keep the JNI side small.
 * Files cross the boundary as: name length u16 BE | UTF-8 name | data.
 */
public class FoqdtActivity extends NativeActivity implements Camera.PreviewCallback, Camera2Source.Sink {
    private static final int PICK = 1, SAVE = 2;

    private Camera camera;
    private SurfaceTexture texture;
    private int width, height;
    private String mode = "";

    // Camera2 path: frames arrive already packed; buffers are recycled because a 4K luma plane
    // is 8 MB and allocating one per frame at 30 fps keeps the GC busy.
    private Camera2Source camera2;
    private byte[] packed, lent;
    private final java.util.ArrayDeque<byte[]> pool = new java.util.ArrayDeque<>();

    @Override public synchronized byte[] buffer(int size) {
        byte[] b = pool.poll();
        return b != null && b.length == size ? b : new byte[size];
    }

    @Override public synchronized void frame(byte[] f) {
        if (packed != null) pool.push(packed);
        packed = f;
        notifyAll();
    }
    private byte[] latest;

    private boolean picking;
    private byte[] picked;
    private byte[] pendingSave;
    private String pendingSaveName;
    private int[] insets = new int[4];

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // Always draw edge to edge behind transparent system bars, and tell Rust how much they cover.
        // (Left to the defaults, whether the surface sat under the status bar varied between launches.)
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS);
        getWindow().setStatusBarColor(Color.TRANSPARENT);
        getWindow().setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= 28) {
            getWindow().getAttributes().layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        }
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        } else {
            getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        getWindow().getDecorView().setOnApplyWindowInsetsListener((v, wi) -> {
            int[] in;
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets i = wi.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
                in = new int[] {i.left, i.top, i.right, i.bottom};
            } else {
                in = new int[] {wi.getSystemWindowInsetLeft(), wi.getSystemWindowInsetTop(),
                    wi.getSystemWindowInsetRight(), wi.getSystemWindowInsetBottom()};
            }
            synchronized (this) { insets = in; }
            return v.onApplyWindowInsets(wi);
        });
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.CAMERA}, 0);
        }
    }

    /** System bar, cutout and keyboard insets in pixels: left, top, right, bottom as i32 BE. */
    public synchronized byte[] insets() {
        ByteBuffer b = ByteBuffer.allocate(16);
        for (int v : insets) b.putInt(v);
        return b.array();
    }

    // ---- camera ----

    /** Returns null on success, else a UTF-8 error message. */
    // ponytail: deprecated android.hardware.Camera is still shipped and needs ~30 lines; Camera2 when it's removed.
    @SuppressWarnings("deprecation")
    public synchronized byte[] startCamera() {
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            runOnUiThread(() -> requestPermissions(new String[] {Manifest.permission.CAMERA}, 0));
            return utf8("camera permission not granted yet; allow it and press Start again");
        }
        stopCamera();
        try {
            camera2 = Camera2Source.open(this, this);
            width = camera2.width;
            height = camera2.height;
            mode = camera2.mode;
            return null;
        } catch (Exception e) {
            android.util.Log.w("foqdt", "Camera2 unavailable, using the legacy camera: " + e);
            camera2 = null;
        }
        try {
            Camera.CameraInfo info = new Camera.CameraInfo();
            int id = 0;
            for (int i = 0; i < Camera.getNumberOfCameras(); i++) {
                Camera.getCameraInfo(i, info);
                if (info.facing == Camera.CameraInfo.CAMERA_FACING_BACK) { id = i; break; }
            }
            camera = Camera.open(id);
            Camera.Parameters p = camera.getParameters();
            // Dense codes need pixels: take the preview size closest to 1080p.
            Camera.Size best = null;
            for (Camera.Size s : p.getSupportedPreviewSizes()) {
                if (best == null || Math.abs(s.width * s.height - 1920 * 1080) < Math.abs(best.width * best.height - 1920 * 1080)) best = s;
            }
            p.setPreviewSize(best.width, best.height);
            // Highest frame rate the camera offers (ranges are fps * 1000).
            int[] fps = null;
            for (int[] r : p.getSupportedPreviewFpsRange()) {
                if (fps == null || r[1] > fps[1] || (r[1] == fps[1] && r[0] > fps[0])) fps = r;
            }
            if (fps != null) p.setPreviewFpsRange(fps[0], fps[1]);
            mode = "Legacy · NV21 · " + (fps == null ? "?" : fps[0] == fps[1] ? fps[1] / 1000 + "" : fps[0] / 1000 + "–" + fps[1] / 1000) + " fps";
            if (p.getSupportedFocusModes().contains(Camera.Parameters.FOCUS_MODE_CONTINUOUS_VIDEO)) {
                p.setFocusMode(Camera.Parameters.FOCUS_MODE_CONTINUOUS_VIDEO);
            }
            // Screens are bright: underexpose a little so white modules don't bleed into black ones,
            // which also shortens the exposure and cuts motion blur.
            p.setExposureCompensation(Math.max(p.getMinExposureCompensation(), Math.round(-1f / p.getExposureCompensationStep())));
            p.setRecordingHint(true); // many drivers only reach their top preview fps in video mode
            camera.setParameters(p);
            width = best.width;
            height = best.height;
            for (int i = 0; i < 3; i++) camera.addCallbackBuffer(new byte[width * height * 3 / 2]);
            texture = new SurfaceTexture(0);
            camera.setPreviewTexture(texture);
            camera.setPreviewCallbackWithBuffer(this);
            camera.startPreview();
            return null;
        } catch (Exception e) {
            stopCamera();
            return utf8(String.valueOf(e));
        }
    }

    @Override
    public synchronized void onPreviewFrame(byte[] data, Camera c) {
        if (latest != null && camera != null) camera.addCallbackBuffer(latest);
        latest = data;
        notifyAll();
    }

    /** Negotiated preview format and frame-rate range, UTF-8. */
    public synchronized byte[] cameraMode() {
        return utf8(mode);
    }

    /** Latest frame as: width u32 BE | height u32 BE | luma plane. Null after 1 s without a frame. */
    public synchronized byte[] takeFrame() throws InterruptedException {
        if (camera2 != null) {
            // The caller has copied the previous frame by now, so its buffer can be reused.
            if (lent != null) { pool.push(lent); lent = null; }
            if (packed == null) wait(1000);
            lent = packed;
            packed = null;
            return lent;
        }
        if (latest == null) wait(1000);
        if (latest == null || camera == null) return null;
        byte[] out = ByteBuffer.allocate(8 + width * height).putInt(width).putInt(height)
            .put(latest, 0, width * height).array();
        camera.addCallbackBuffer(latest);
        latest = null;
        return out;
    }

    public synchronized byte[] stopCamera() {
        if (camera2 != null) {
            camera2.close();
            camera2 = null;
        }
        packed = null;
        lent = null;
        pool.clear();
        if (camera != null) {
            camera.setPreviewCallbackWithBuffer(null);
            camera.stopPreview();
            camera.release();
            camera = null;
        }
        if (texture != null) { texture.release(); texture = null; }
        latest = null;
        notifyAll();
        return null;
    }

    /** Status/navigation bar icons: light icons for a dark theme (arg {1}) and dark icons otherwise. */
    public byte[] systemBars(byte[] dark) {
        boolean d = dark.length > 0 && dark[0] != 0;
        runOnUiThread(() -> {
            if (Build.VERSION.SDK_INT >= 30) {
                int light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                getWindow().getInsetsController().setSystemBarsAppearance(d ? 0 : light, light);
            } else {
                View v = getWindow().getDecorView();
                int light = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                v.setSystemUiVisibility(d ? v.getSystemUiVisibility() & ~light : v.getSystemUiVisibility() | light);
            }
        });
        return null;
    }

    /**
     * NativeActivity.onDestroy blocks until the Rust thread exits, but eframe's loop never does,
     * so finishing or recreating the activity froze the app (ANR). End the process instead.
     */
    @Override
    protected void onDestroy() {
        android.os.Process.killProcess(android.os.Process.myPid());
        super.onDestroy();
    }

    @Override
    protected void onPause() {
        stopCamera();
        super.onPause();
    }

    // ---- files ----

    public synchronized byte[] pickFile() {
        picking = true;
        picked = null;
        runOnUiThread(() -> {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
            startActivityForResult(i, PICK);
        });
        return null;
    }

    /** Null while the picker is open, empty if cancelled, else an encoded file. */
    public synchronized byte[] pickResult() {
        if (picking) return null;
        byte[] out = picked == null ? new byte[0] : picked;
        picked = null;
        return out;
    }

    public synchronized byte[] saveFile(byte[] encoded) {
        int n = ((encoded[0] & 0xff) << 8) | (encoded[1] & 0xff);
        pendingSaveName = new String(encoded, 2, n, StandardCharsets.UTF_8);
        pendingSave = Arrays.copyOfRange(encoded, 2 + n, encoded.length);
        runOnUiThread(() -> {
            Intent i = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType("application/octet-stream").putExtra(Intent.EXTRA_TITLE, pendingSaveName);
            startActivityForResult(i, SAVE);
        });
        return null;
    }

    /** Write to cache/received and open it with the default app. Null on success, else an error (UTF-8). */
    public byte[] openFile(byte[] encoded) {
        int n = ((encoded[0] & 0xff) << 8) | (encoded[1] & 0xff);
        // The name comes off the wire: keep only the last path component.
        String base = new File(new String(encoded, 2, n, StandardCharsets.UTF_8)).getName();
        final String name = base.isEmpty() ? "received" : base;
        try {
            File dir = new File(getCacheDir(), "received");
            dir.mkdirs();
            File f = new File(dir, name);
            try (FileOutputStream os = new FileOutputStream(f)) { os.write(encoded, 2 + n, encoded.length - 2 - n); }
            String ext = name.lastIndexOf('.') >= 0 ? name.substring(name.lastIndexOf('.') + 1).toLowerCase() : "";
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
            Intent i = new Intent(Intent.ACTION_VIEW)
                .setDataAndType(ReceivedProvider.uri(this, name), mime != null ? mime : "application/octet-stream")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
            runOnUiThread(() -> {
                try { startActivity(Intent.createChooser(i, name).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
                catch (Exception e) { toast("No app can open " + name); }
            });
            return null;
        } catch (Exception e) {
            return utf8(String.valueOf(e));
        }
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        Uri uri = (result == RESULT_OK && data != null) ? data.getData() : null;
        new Thread(() -> {
            try {
                if (request == PICK) {
                    byte[] out = uri == null ? null : encode(displayName(uri), readAll(uri));
                    synchronized (this) { picked = out; picking = false; }
                } else if (request == SAVE && uri != null) {
                    byte[] bytes;
                    synchronized (this) { bytes = pendingSave; pendingSave = null; }
                    try (OutputStream os = getContentResolver().openOutputStream(uri)) { os.write(bytes); }
                    toast("Saved " + pendingSaveName);
                }
            } catch (Exception e) {
                synchronized (this) { picking = false; }
                toast(String.valueOf(e));
            }
        }).start();
    }

    private byte[] readAll(Uri uri) throws Exception {
        try (InputStream is = getContentResolver().openInputStream(uri)) {
            ByteArrayOutputStream bos = new ByteArrayOutputStream();
            byte[] buf = new byte[1 << 16];
            for (int n; (n = is.read(buf)) > 0; ) bos.write(buf, 0, n);
            return bos.toByteArray();
        }
    }

    private String displayName(Uri uri) {
        try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) return c.getString(0);
        }
        return "file";
    }

    private static byte[] encode(String name, byte[] data) {
        byte[] n = name.getBytes(StandardCharsets.UTF_8);
        return ByteBuffer.allocate(2 + n.length + data.length).putShort((short) n.length).put(n).put(data).array();
    }

    private static byte[] utf8(String s) {
        return s.getBytes(StandardCharsets.UTF_8);
    }

    private void toast(String msg) {
        runOnUiThread(() -> Toast.makeText(this, msg, Toast.LENGTH_SHORT).show());
    }
}
