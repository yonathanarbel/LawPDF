package com.lawpdf.mobile;

import android.app.Activity;
import android.app.Application;
import android.content.Context;
import android.content.Intent;
import android.graphics.Paint;
import android.graphics.pdf.PdfDocument;
import android.net.Uri;
import android.os.Bundle;
import android.os.SystemClock;
import android.test.InstrumentationTestCase;
import java.io.File;
import java.io.FileOutputStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Collections;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

/** Runs on a disposable emulator against real AtomicFile and activity lifecycles. */
@SuppressWarnings("deprecation")
public final class PersistenceInstrumentationTest extends InstrumentationTestCase {
    private Context context() { return getInstrumentation().getTargetContext(); }

    private File pdf(String text) throws Exception {
        File output = File.createTempFile("persistence-qa-", ".pdf", context().getCacheDir());
        PdfDocument document = new PdfDocument();
        try {
            PdfDocument.Page page = document.startPage(new PdfDocument.PageInfo.Builder(300, 400, 1).create());
            page.getCanvas().drawText(text, 20, 80, new Paint());
            document.finishPage(page);
            try (FileOutputStream stream = new FileOutputStream(output)) { document.writeTo(stream); stream.getFD().sync(); }
        } finally { document.close(); }
        return output;
    }

    private AnnotationStore.Stroke mark() {
        AnnotationStore.Stroke stroke = new AnnotationStore.Stroke(0, AnnotationStore.Tool.HIGHLIGHT, 0xffffff00, 0.02f);
        stroke.add(0.1f, 0.2f); stroke.add(0.8f, 0.2f);
        return stroke;
    }

    public void testJournalRestoresAfterRecreationAndRejectsDamagedData() throws Exception {
        File directory = new File(context().getCacheDir(), "journal-qa-" + System.nanoTime());
        String revision = new String(new char[64]).replace('\0', 'a');
        AnnotationJournal first = new AnnotationJournal(directory, revision);
        first.write(Collections.singletonList(mark()));
        AnnotationJournal reopened = new AnnotationJournal(directory, revision);
        assertEquals(1, reopened.read(1).size());
        assertEquals(2, reopened.read(1).get(0).points.size());
        File journal = new File(directory, revision + ".marks");
        try (FileOutputStream stream = new FileOutputStream(journal)) { stream.write(new byte[]{1, 2, 3}); }
        try { reopened.read(1); fail("Damaged journals must be retained and reported"); }
        catch (java.io.IOException expected) { assertEquals(3, journal.length()); }
        assertTrue(journal.delete()); assertTrue(directory.delete());
    }

    public void testMissingProviderFileUsesTheExactRecoverySource() throws Exception {
        File original = pdf("LawPDF provider recovery QA");
        Uri uri = Uri.fromFile(original);
        DocumentSnapshot first = DocumentSnapshot.open(context(), context().getContentResolver(), uri);
        AnnotationJournal journal = new AnnotationJournal(new File(context().getFilesDir(), "annotation-journals"), first.revision);
        journal.write(Collections.singletonList(mark()));
        assertTrue(original.delete());
        DocumentSnapshot recovered = DocumentSnapshot.open(context(), context().getContentResolver(), uri);
        assertTrue(recovered.recovered);
        assertEquals(first.revision, recovered.revision);
        assertTrue(recovered.file.isFile());
        assertEquals(1, journal.read(1).size());
    }

    public void testChangedProviderBytesDoNotBorrowOldMarks() throws Exception {
        File original = pdf("First document revision");
        Uri uri = Uri.fromFile(original);
        DocumentSnapshot first = DocumentSnapshot.open(context(), context().getContentResolver(), uri);
        File changed = pdf("A different document revision");
        try (java.io.InputStream input = new java.io.FileInputStream(changed); FileOutputStream output = new FileOutputStream(original)) {
            byte[] buffer = new byte[8192]; int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
        }
        DocumentSnapshot second = DocumentSnapshot.open(context(), context().getContentResolver(), uri);
        assertFalse(first.revision.equals(second.revision));
        assertTrue(first.file.isFile());
        assertTrue(second.file.isFile());
        assertTrue(original.delete()); assertTrue(changed.delete());
    }

    private PdfPageList pages(Activity activity) throws Exception {
        Field field = MainActivity.class.getDeclaredField("pdfPageList"); field.setAccessible(true);
        return (PdfPageList) field.get(activity);
    }

    private AnnotationStore store(PdfPageList pages) throws Exception {
        Field field = PdfPageList.class.getDeclaredField("annotations"); field.setAccessible(true);
        return (AnnotationStore) field.get(pages);
    }

    private int readyCount(Activity activity) {
        AtomicInteger count = new AtomicInteger(-1);
        getInstrumentation().runOnMainSync(() -> {
            try { PdfPageList pages = pages(activity); if (pages != null && pages.getAdapter() != null) count.set(store(pages).size()); }
            catch (Exception error) { throw new AssertionError(error); }
        });
        return count.get();
    }

    private void awaitReady(Activity activity) {
        long deadline = SystemClock.uptimeMillis() + 20000;
        while (readyCount(activity) < 0 && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50);
        assertTrue("PDF did not open within 20 seconds", readyCount(activity) >= 0);
    }

    public void testActivityRecreationRestoresCommittedMarks() throws Exception {
        File original = pdf("LawPDF activity recreation QA " + System.nanoTime());
        MainActivity activity = (MainActivity) getInstrumentation().startActivitySync(new Intent(context(), MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        Application application = (Application) context().getApplicationContext();
        AtomicReference<MainActivity> recreated = new AtomicReference<>();
        Application.ActivityLifecycleCallbacks callbacks = new Application.ActivityLifecycleCallbacks() {
            public void onActivityCreated(Activity value, Bundle state) { if (value instanceof MainActivity && value != activity) recreated.set((MainActivity) value); }
            public void onActivityStarted(Activity value) {} public void onActivityResumed(Activity value) {}
            public void onActivityPaused(Activity value) {} public void onActivityStopped(Activity value) {}
            public void onActivitySaveInstanceState(Activity value, Bundle state) {} public void onActivityDestroyed(Activity value) {}
        };
        application.registerActivityLifecycleCallbacks(callbacks);
        try {
            getInstrumentation().runOnMainSync(() -> {
                try { Method open = MainActivity.class.getDeclaredMethod("openDocument", Uri.class, String.class); open.setAccessible(true); open.invoke(activity, Uri.fromFile(original), "application/pdf"); }
                catch (Exception error) { throw new AssertionError(error); }
            });
            awaitReady(activity);
            getInstrumentation().runOnMainSync(() -> {
                try { store(pages(activity)).commit(mark()); }
                catch (Exception error) { throw new AssertionError(error); }
            });
            assertEquals(1, readyCount(activity));
            getInstrumentation().runOnMainSync(activity::recreate);
            long deadline = SystemClock.uptimeMillis() + 20000;
            while (recreated.get() == null && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50);
            assertNotNull("Activity was not recreated", recreated.get());
            awaitReady(recreated.get());
            assertEquals(1, readyCount(recreated.get()));
        } finally {
            application.unregisterActivityLifecycleCallbacks(callbacks);
            getInstrumentation().runOnMainSync(() -> { if (recreated.get() != null) recreated.get().finish(); else activity.finish(); });
            original.delete();
        }
    }
}
