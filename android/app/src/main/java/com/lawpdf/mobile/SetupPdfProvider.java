package com.lawpdf.mobile;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.graphics.pdf.PdfDocument;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * Serves one harmless generated PDF used solely to invoke Android's standard
 * "Open with" resolver. Temporary URI permission is granted by MainActivity.
 */
public final class SetupPdfProvider extends ContentProvider {
    static final String AUTHORITY = "com.lawpdf.mobile.setup";
    static final Uri SETUP_URI =
            Uri.parse("content://" + AUTHORITY + "/choose-lawpdf.pdf");
    private static final String FILE_NAME = "choose-lawpdf.pdf";

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        return "application/pdf";
    }

    @Override
    public Cursor query(
            Uri uri,
            String[] projection,
            String selection,
            String[] selectionArgs,
            String sortOrder) {
        String[] columns = projection == null
                ? new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}
                : projection;
        MatrixCursor cursor = new MatrixCursor(columns, 1);
        MatrixCursor.RowBuilder row = cursor.newRow();
        File file = sampleFile();
        for (String column : columns) {
            if (OpenableColumns.DISPLAY_NAME.equals(column)) {
                row.add(FILE_NAME);
            } else if (OpenableColumns.SIZE.equals(column)) {
                row.add(file.exists() ? file.length() : 0L);
            } else {
                row.add(null);
            }
        }
        return cursor;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!SETUP_URI.getPath().equals(uri.getPath()) || mode == null || !mode.startsWith("r")) {
            throw new FileNotFoundException("Unknown LawPDF setup document.");
        }
        try {
            File file = ensureSamplePdf();
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
        } catch (IOException error) {
            FileNotFoundException wrapped =
                    new FileNotFoundException("Could not create the LawPDF setup document.");
            wrapped.initCause(error);
            throw wrapped;
        }
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("Read-only provider.");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(
            Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }

    private synchronized File ensureSamplePdf() throws IOException {
        File file = sampleFile();
        if (file.exists() && file.length() > 0) return file;
        File parent = file.getParentFile();
        if (parent != null && !parent.exists() && !parent.mkdirs()) {
            throw new IOException("Could not create setup cache.");
        }

        PdfDocument document = new PdfDocument();
        try {
            PdfDocument.PageInfo info =
                    new PdfDocument.PageInfo.Builder(612, 792, 1).create();
            PdfDocument.Page page = document.startPage(info);
            Canvas canvas = page.getCanvas();
            canvas.drawColor(Color.rgb(248, 246, 240));

            Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            paint.setColor(Color.rgb(20, 45, 70));
            canvas.drawRoundRect(new RectF(56, 64, 556, 232), 24, 24, paint);
            paint.setColor(Color.rgb(235, 183, 72));
            canvas.drawCircle(112, 124, 28, paint);
            paint.setColor(Color.rgb(20, 45, 70));
            paint.setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD));
            paint.setTextSize(30);
            canvas.drawText("L", 102, 134, paint);

            paint.setColor(Color.WHITE);
            paint.setTextSize(18);
            paint.setLetterSpacing(0.12f);
            canvas.drawText("LAWPDF", 160, 112, paint);
            paint.setLetterSpacing(0f);
            paint.setTextSize(28);
            canvas.drawText("Make LawPDF your PDF app", 160, 158, paint);

            paint.setColor(Color.rgb(30, 43, 54));
            paint.setTextSize(25);
            canvas.drawText("Choose LawPDF in Android's app list.", 64, 310, paint);
            paint.setColor(Color.rgb(91, 108, 122));
            paint.setTextSize(20);
            canvas.drawText("Then tap Always to use it for future PDFs.", 64, 354, paint);
            canvas.drawText("You can change this later in Android Settings.", 64, 392, paint);

            paint.setColor(Color.rgb(54, 105, 162));
            canvas.drawRoundRect(new RectF(64, 470, 548, 554), 18, 18, paint);
            paint.setColor(Color.WHITE);
            paint.setTextSize(23);
            paint.setTypeface(Typeface.create(Typeface.DEFAULT, Typeface.BOLD));
            canvas.drawText("A private, focused PDF workspace", 106, 522, paint);

            document.finishPage(page);
            try (FileOutputStream output = new FileOutputStream(file)) {
                document.writeTo(output);
            }
        } finally {
            document.close();
        }
        return file;
    }

    private File sampleFile() {
        if (getContext() == null) return new File(FILE_NAME);
        return new File(new File(getContext().getCacheDir(), "default-setup"), FILE_NAME);
    }
}
