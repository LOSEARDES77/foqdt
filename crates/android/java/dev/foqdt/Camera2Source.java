package dev.foqdt;

import android.content.Context;
import android.graphics.ImageFormat;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Range;
import android.util.Rational;
import android.util.Size;
import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/**
 * Back camera through Camera2: up to 4K luma frames at the sensor's full video rate.
 * The deprecated android.hardware.Camera API caps most phones at a 1080p preview,
 * which is too coarse for four dense QR codes per frame.
 */
final class Camera2Source {
    interface Sink {
        /** A reusable buffer of at least `size` bytes. */
        byte[] buffer(int size);
        /** Frame packed as: width u32 BE | height u32 BE | luma plane. */
        void frame(byte[] packed);
    }

    private static final long FRAME_30FPS_NS = 33_400_000L;

    final String mode;
    final int width, height;
    private final HandlerThread thread;
    private final Handler handler;
    private final CameraDevice device;
    private final CameraCaptureSession session;
    private final ImageReader reader;

    private Camera2Source(String mode, Size size, HandlerThread thread, Handler handler, CameraDevice device, CameraCaptureSession session, ImageReader reader) {
        this.handler = handler;
        this.mode = mode;
        this.width = size.getWidth();
        this.height = size.getHeight();
        this.thread = thread;
        this.device = device;
        this.session = session;
        this.reader = reader;
    }

    /** Opens the back camera and starts streaming into `sink`; throws when Camera2 can't do it. */
    static Camera2Source open(Context ctx, Sink sink) throws Exception {
        CameraManager mgr = ctx.getSystemService(CameraManager.class);
        String id = null;
        CameraCharacteristics chars = null;
        for (String c : mgr.getCameraIdList()) {
            CameraCharacteristics ch = mgr.getCameraCharacteristics(c);
            Integer facing = ch.get(CameraCharacteristics.LENS_FACING);
            Integer level = ch.get(CameraCharacteristics.INFO_SUPPORTED_HARDWARE_LEVEL);
            if (facing != null && facing == CameraCharacteristics.LENS_FACING_BACK
                && level != null && level != CameraCharacteristics.INFO_SUPPORTED_HARDWARE_LEVEL_LEGACY) {
                id = c;
                chars = ch;
                break;
            }
        }
        if (id == null) throw new IllegalStateException("no Camera2 back camera");

        // Largest YUV size up to 4K that still streams at 30 fps.
        StreamConfigurationMap map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
        Size best = null;
        for (Size s : map.getOutputSizes(ImageFormat.YUV_420_888)) {
            long area = (long) s.getWidth() * s.getHeight();
            if (area > 3840L * 2160 || map.getOutputMinFrameDuration(ImageFormat.YUV_420_888, s) > FRAME_30FPS_NS) continue;
            boolean wide = s.getWidth() * 9 == s.getHeight() * 16;
            if (best == null) { best = s; continue; }
            long bestArea = (long) best.getWidth() * best.getHeight();
            boolean bestWide = best.getWidth() * 9 == best.getHeight() * 16;
            if (area > bestArea || (area == bestArea && wide && !bestWide)) best = s;
        }
        if (best == null) throw new IllegalStateException("no YUV size at 30 fps");
        long minFrame = map.getOutputMinFrameDuration(ImageFormat.YUV_420_888, best);
        int maxFps = minFrame > 0 ? (int) (1_000_000_000L / minFrame) : 30;

        // Highest frame-rate range the stream can hold; prefer fixed ranges (lower bound high).
        Range<Integer> fps = null;
        Range<Integer>[] ranges = chars.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES);
        if (ranges != null) {
            for (Range<Integer> r : ranges) {
                if (r.getUpper() > maxFps) continue;
                if (fps == null || r.getUpper() > fps.getUpper() || (r.getUpper().equals(fps.getUpper()) && r.getLower() > fps.getLower())) fps = r;
            }
        }

        HandlerThread thread = new HandlerThread("foqdt-camera");
        thread.start();
        Handler handler = new Handler(thread.getLooper());
        final int w = best.getWidth(), h = best.getHeight();
        ImageReader reader = ImageReader.newInstance(w, h, ImageFormat.YUV_420_888, 3);
        reader.setOnImageAvailableListener(r -> {
            Image img = r.acquireLatestImage();
            if (img == null) return;
            try {
                Image.Plane y = img.getPlanes()[0];
                ByteBuffer src = y.getBuffer();
                int stride = y.getRowStride();
                byte[] out = sink.buffer(8 + w * h);
                ByteBuffer.wrap(out, 0, 8).putInt(w).putInt(h);
                if (stride == w) {
                    src.get(out, 8, w * h);
                } else {
                    for (int row = 0; row < h; row++) {
                        src.position(row * stride);
                        src.get(out, 8 + row * w, w);
                    }
                }
                sink.frame(out);
            } catch (IllegalStateException closing) {
                // The reader is shutting down; drop this frame.
            } finally {
                img.close();
            }
        }, handler);

        final CameraDevice[] dev = new CameraDevice[1];
        final Exception[] err = new Exception[1];
        CountDownLatch opened = new CountDownLatch(1);
        mgr.openCamera(id, new CameraDevice.StateCallback() {
            @Override public void onOpened(CameraDevice d) { dev[0] = d; opened.countDown(); }
            @Override public void onDisconnected(CameraDevice d) { d.close(); opened.countDown(); }
            @Override public void onError(CameraDevice d, int e) { d.close(); err[0] = new IllegalStateException("camera error " + e); opened.countDown(); }
        }, handler);
        if (!opened.await(4, TimeUnit.SECONDS) || dev[0] == null) {
            reader.close();
            thread.quitSafely();
            throw err[0] != null ? err[0] : new IllegalStateException("camera did not open");
        }

        final CameraCaptureSession[] ses = new CameraCaptureSession[1];
        CountDownLatch configured = new CountDownLatch(1);
        dev[0].createCaptureSession(Collections.singletonList(reader.getSurface()), new CameraCaptureSession.StateCallback() {
            @Override public void onConfigured(CameraCaptureSession s) { ses[0] = s; configured.countDown(); }
            @Override public void onConfigureFailed(CameraCaptureSession s) { configured.countDown(); }
        }, handler);
        if (!configured.await(4, TimeUnit.SECONDS) || ses[0] == null) {
            dev[0].close();
            reader.close();
            thread.quitSafely();
            throw new IllegalStateException("capture session failed");
        }

        CaptureRequest.Builder req = dev[0].createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
        req.addTarget(reader.getSurface());
        req.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO);
        if (fps != null) req.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, fps);
        // Screens are bright: -1 EV keeps white modules from bleeding and shortens the exposure.
        Rational step = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP);
        Range<Integer> comp = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE);
        if (step != null && comp != null && step.floatValue() > 0) {
            req.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, Math.max(comp.getLower(), Math.round(-1f / step.floatValue())));
        }
        ses[0].setRepeatingRequest(req.build(), null, handler);

        String range = fps == null ? "?" : fps.getLower().equals(fps.getUpper()) ? "" + fps.getUpper() : fps.getLower() + "–" + fps.getUpper();
        return new Camera2Source("Camera2 · YUV · " + range + " fps", best, thread, handler, dev[0], ses[0], reader);
    }

    /**
     * Close on the camera thread: frames are copied there, and closing the reader from another
     * thread mid-copy throws "buffer is inaccessible" and kills the app.
     */
    void close() {
        CountDownLatch done = new CountDownLatch(1);
        handler.post(() -> {
            try { session.close(); } catch (Exception ignored) {}
            try { device.close(); } catch (Exception ignored) {}
            try { reader.close(); } catch (Exception ignored) {}
            done.countDown();
        });
        try { done.await(2, TimeUnit.SECONDS); } catch (InterruptedException ignored) {}
        thread.quitSafely();
    }
}
