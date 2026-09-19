package com.lawpdf.mobile;

import android.content.ContentResolver;
import android.content.Context;
import android.net.Uri;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;

/** Annotation identity follows exact document bytes, never a mutable provider URI. */
final class DocumentSnapshot {
    static final long MAX_BYTES = 128L * 1024 * 1024;
    final File file;
    final String revision;
    final boolean recovered;
    private DocumentSnapshot(File file, String revision, boolean recovered) { this.file = file; this.revision = revision; this.recovered = recovered; }

    static DocumentSnapshot open(Context context, ContentResolver resolver, Uri uri) throws IOException {
        File directory = new File(context.getFilesDir(), "document-sources");
        if (!directory.isDirectory() && !directory.mkdirs()) throw new IOException("Could not preserve the source PDF.");
        File temporary = File.createTempFile("opening-", ".pdf", directory);
        try {
            MessageDigest digest = digest();
            try (InputStream input = resolver.openInputStream(uri);
                 FileOutputStream output = new FileOutputStream(temporary)) {
                if (input == null) throw new IOException("The document provider returned no PDF data.");
                byte[] buffer = new byte[64 * 1024];
                long total = 0;
                int count;
                while ((count = input.read(buffer)) != -1) {
                    total += count;
                    if (total > MAX_BYTES) throw new IOException("This PDF exceeds the 128 MB Android document limit.");
                    digest.update(buffer, 0, count); output.write(buffer, 0, count);
                }
                output.getFD().sync();
            }
            String revision = hex(digest.digest());
            File destination = new File(directory, revision + ".pdf");
            if (destination.exists()) {
                if (!hash(destination).equals(revision)) throw new IOException("The local PDF copy is damaged. Your annotations were kept.");
            } else {
                long used = 0;
                File[] sources = directory.listFiles();
                if (sources == null) throw new IOException("Could not inspect local recovery storage.");
                for (File source : sources) used += source.length();
                if (used > 1024L * 1024 * 1024) throw new IOException("Local PDF recovery reached its 1 GB limit. Export your marked documents before clearing LawPDF storage in Android settings.");
                if (!temporary.renameTo(destination)) throw new IOException("Could not finish preserving the PDF.");
            }
            if (!context.getSharedPreferences("document-revisions", Context.MODE_PRIVATE).edit().putString(uri.toString(), revision).commit()) {
                throw new IOException("Could not remember this document for recovery.");
            }
            return new DocumentSnapshot(destination, revision, false);
        } catch (SecurityException error) {
            return recover(context, directory, uri, error);
        } catch (java.io.FileNotFoundException error) {
            return recover(context, directory, uri, error);
        } finally {
            if (temporary.exists()) temporary.delete();
        }
    }

    private static DocumentSnapshot recover(Context context, File directory, Uri uri, Exception cause) throws IOException {
        String revision = context.getSharedPreferences("document-revisions", Context.MODE_PRIVATE).getString(uri.toString(), "");
        if (!revision.matches("[a-f0-9]{64}")) throw new IOException("The PDF is unavailable. Open it again through the document picker.", cause);
        File snapshot = new File(directory, revision + ".pdf");
        if (!snapshot.isFile() || !hash(snapshot).equals(revision)) throw new IOException("The provider and the local PDF copy are unavailable. Your annotation records were kept.", cause);
        return new DocumentSnapshot(snapshot, revision, true);
    }

    private static String hash(File file) throws IOException {
        if (file.length() > MAX_BYTES) throw new IOException("The local PDF copy exceeds the size limit.");
        MessageDigest digest = digest();
        try (InputStream input = new FileInputStream(file)) {
            byte[] buffer = new byte[64 * 1024]; int count;
            while ((count = input.read(buffer)) != -1) digest.update(buffer, 0, count);
        }
        return hex(digest.digest());
    }
    private static MessageDigest digest() { try { return MessageDigest.getInstance("SHA-256"); } catch (NoSuchAlgorithmException impossible) { throw new AssertionError(impossible); } }
    private static String hex(byte[] bytes) { StringBuilder value = new StringBuilder(64); for (byte item : bytes) value.append(String.format(java.util.Locale.ROOT, "%02x", item & 0xff)); return value.toString(); }
}
