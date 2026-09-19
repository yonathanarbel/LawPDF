package com.lawpdf.mobile;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Process;
import android.os.SystemClock;
import android.test.InstrumentationTestRunner;
import android.os.Bundle;
import java.io.File;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.concurrent.atomic.AtomicInteger;

/** The release QA driver runs these phases in different, force-stopped processes. */
@SuppressWarnings("deprecation")
public final class ProcessDeathInstrumentation extends InstrumentationTestRunner {
    private String phase;
    @Override public void onCreate(Bundle arguments) { phase = arguments.getString("phase", ""); if (phase.isEmpty()) super.onCreate(arguments); else start(); }
    @Override public void onStart() {
        if (phase.isEmpty()) { super.onStart(); return; }
        Bundle result = new Bundle();
        try {
            if ("prepare".equals(phase)) prepare();
            else if ("restore".equals(phase)) restore();
            else throw new AssertionError("Select prepare or restore");
            result.putString("stream", "LAWPDF_PROCESS_DEATH_OK " + phase + " pid=" + Process.myPid() + "\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", "LAWPDF_PROCESS_DEATH_FAILED " + error.toString() + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
    private static void assertTrue(String message, boolean value) { if (!value) throw new AssertionError(message); }
    private static void assertTrue(boolean value) { assertTrue("Expected success", value); }
    private static void assertEquals(String message, int expected, int actual) { assertTrue(message, expected == actual); }
    private static void assertEquals(int expected, int actual) { assertEquals("Unexpected mark count", expected, actual); }
    private Context context() { return getTargetContext(); }
    private AnnotationStore store(Activity activity) throws Exception {
        Field pages = MainActivity.class.getDeclaredField("pdfPageList"); pages.setAccessible(true);
        Field marks = PdfPageList.class.getDeclaredField("annotations"); marks.setAccessible(true);
        PdfPageList list = (PdfPageList) pages.get(activity);
        return list != null && list.getAdapter() != null ? (AnnotationStore) marks.get(list) : null;
    }
    private int count(Activity activity) {
        AtomicInteger value = new AtomicInteger(-1);
        runOnMainSync(() -> {
            try { AnnotationStore marks = store(activity); if (marks != null) value.set(marks.size()); }
            catch (Exception error) { throw new AssertionError(error); }
        });
        return value.get();
    }
    private Activity open(Uri uri) {
        Activity activity = startActivitySync(new Intent(context(), MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        runOnMainSync(() -> {
            try { Method open = MainActivity.class.getDeclaredMethod("openDocument", Uri.class, String.class); open.setAccessible(true); open.invoke(activity, uri, "application/pdf"); }
            catch (Exception error) { throw new AssertionError(error); }
        });
        long deadline = SystemClock.uptimeMillis() + 20000;
        while (count(activity) < 0 && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50);
        assertTrue("Document did not open", count(activity) >= 0);
        return activity;
    }
    private void prepare() throws Exception {
        File file = File.createTempFile("process-death-qa-", ".pdf", context().getFilesDir());
        android.graphics.pdf.PdfDocument pdf = new android.graphics.pdf.PdfDocument();
        try {
            android.graphics.pdf.PdfDocument.Page page = pdf.startPage(new android.graphics.pdf.PdfDocument.PageInfo.Builder(300, 400, 1).create());
            page.getCanvas().drawText("LawPDF process death QA", 20, 80, new android.graphics.Paint()); pdf.finishPage(page);
            try (java.io.FileOutputStream out = new java.io.FileOutputStream(file)) { pdf.writeTo(out); out.getFD().sync(); }
        } finally { pdf.close(); }
        Uri uri = Uri.fromFile(file);
        Activity activity = open(uri);
        try {
            runOnMainSync(() -> {
                try {
                    AnnotationStore.Stroke mark = new AnnotationStore.Stroke(0, AnnotationStore.Tool.HIGHLIGHT, 0xffffff00, 0.02f);
                    mark.add(0.1f, 0.2f); mark.add(0.8f, 0.2f); store(activity).commit(mark);
                } catch (Exception error) { throw new AssertionError(error); }
            });
            assertEquals(1, count(activity));
            assertTrue(context().getSharedPreferences("process-death-qa", 0).edit().putString("uri", uri.toString()).putInt("pid", Process.myPid()).commit());
        } finally { runOnMainSync(activity::finish); }
    }
    private void restore() throws Exception {
        android.content.SharedPreferences state = context().getSharedPreferences("process-death-qa", 0);
        assertTrue("Run the prepare phase in a separate process first", state.getInt("pid", Process.myPid()) != Process.myPid());
        Uri uri = Uri.parse(state.getString("uri", ""));
        Activity activity = open(uri);
        try { assertEquals("Committed marks must survive force-stop", 1, count(activity)); }
        finally { runOnMainSync(activity::finish); new File(uri.getPath()).delete(); state.edit().clear().commit(); }
    }
}
