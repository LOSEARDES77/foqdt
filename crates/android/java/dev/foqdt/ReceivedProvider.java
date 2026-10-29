package dev.foqdt;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileNotFoundException;

/**
 * Serves cache/received/NAME as content://dev.foqdt.received/NAME so other apps can open
 * received files (a minimal stand-in for androidx FileProvider, which we don't ship).
 */
public class ReceivedProvider extends ContentProvider {
    static Uri uri(Context ctx, String name) {
        return new Uri.Builder().scheme("content").authority("dev.foqdt.received").appendPath(name).build();
    }

    private File file(Uri uri) throws FileNotFoundException {
        String name = new File(uri.getLastPathSegment() == null ? "" : uri.getLastPathSegment()).getName();
        File f = new File(new File(getContext().getCacheDir(), "received"), name);
        if (name.isEmpty() || !f.isFile()) throw new FileNotFoundException(uri.toString());
        return f;
    }

    @Override public boolean onCreate() { return true; }

    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override public String getType(Uri uri) {
        String n = uri.getLastPathSegment() == null ? "" : uri.getLastPathSegment();
        String ext = n.lastIndexOf('.') >= 0 ? n.substring(n.lastIndexOf('.') + 1).toLowerCase() : "";
        String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
        return mime != null ? mime : "application/octet-stream";
    }

    @Override public Cursor query(Uri uri, String[] projection, String sel, String[] args, String sort) {
        MatrixCursor c = new MatrixCursor(new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        try { File f = file(uri); c.addRow(new Object[] {f.getName(), f.length()}); } catch (FileNotFoundException ignored) {}
        return c;
    }

    @Override public Uri insert(Uri uri, ContentValues v) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String s, String[] a) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri, ContentValues v, String s, String[] a) { throw new UnsupportedOperationException(); }
}
