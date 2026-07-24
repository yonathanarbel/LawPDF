package com.lawpdf.mobile;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ComponentCallbacks2;
import android.content.Intent;
import android.content.pm.ResolveInfo;
import android.database.Cursor;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.provider.Settings;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.Toast;

import java.io.InputStream;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;

public final class MainActivity extends Activity {
    private static final int OPEN_DOCUMENT_REQUEST = 41;
    private static final int SAVE_DOCUMENT_REQUEST = 42;
    private static final String DOCX_MIME =
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    private static final String STATE_DOCUMENT_URI = "document_uri";
    private static final String STATE_DOCUMENT_MIME = "document_mime";
    private static final String STATE_DOCUMENT_NAME = "document_name";
    private static final String STATE_CHROME_COLLAPSED = "chrome_collapsed";
    private static final String STATE_TOOLS_EXPANDED = "tools_expanded";
    private static final String STATE_INK_INDEX = "ink_index";
    private static final int[] INK_COLORS = {
            Color.rgb(20, 45, 80),
            Color.rgb(190, 35, 35),
            Color.rgb(20, 95, 185),
            Color.BLACK
    };

    private final ExecutorService documentExecutor = Executors.newSingleThreadExecutor();
    private final AtomicInteger documentGeneration = new AtomicInteger();
    private FrameLayout content;
    private ReaderChrome chrome;
    private PdfPageList pdfPageList;
    private Uri currentDocumentUri;
    private String currentDocumentMime;
    private String currentDocumentName = "Document.pdf";
    private int inkColorIndex;
    private boolean waitingForDefaultSettings;
    private boolean leftForDefaultSettings;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            getWindow().setDecorFitsSystemWindows(false);
        }
        if (state != null) inkColorIndex = state.getInt(STATE_INK_INDEX, 0);
        setContentView(createLayout());
        chrome.setInkColor(INK_COLORS[inkColorIndex], getColorName(inkColorIndex));

        boolean handledIntent = handleViewIntent(getIntent());
        if (!handledIntent && state != null) {
            String savedUri = state.getString(STATE_DOCUMENT_URI);
            if (savedUri != null) {
                currentDocumentName = state.getString(STATE_DOCUMENT_NAME, "Document.pdf");
                openDocument(Uri.parse(savedUri), state.getString(STATE_DOCUMENT_MIME));
            }
            chrome.setCollapsed(state.getBoolean(STATE_CHROME_COLLAPSED, false));
            chrome.setToolsExpanded(state.getBoolean(STATE_TOOLS_EXPANDED, false));
        }
        updateDefaultState();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (!isSetupIntent(intent)) setIntent(intent);
        handleViewIntent(intent);
    }

    @Override
    protected void onResume() {
        super.onResume();
        updateDefaultState();
        if (waitingForDefaultSettings && leftForDefaultSettings) {
            waitingForDefaultSettings = false;
            leftForDefaultSettings = false;
            chrome.postDelayed(this::continueDefaultSetupAfterSettings, 300);
        }
    }

    @Override
    protected void onPause() {
        if (waitingForDefaultSettings) leftForDefaultSettings = true;
        super.onPause();
    }

    @Override
    protected void onSaveInstanceState(Bundle state) {
        if (currentDocumentUri != null) {
            state.putString(STATE_DOCUMENT_URI, currentDocumentUri.toString());
            state.putString(STATE_DOCUMENT_MIME, currentDocumentMime);
            state.putString(STATE_DOCUMENT_NAME, currentDocumentName);
        }
        state.putBoolean(STATE_CHROME_COLLAPSED, chrome.isCollapsed());
        state.putBoolean(STATE_TOOLS_EXPANDED, chrome.areToolsExpanded());
        state.putInt(STATE_INK_INDEX, inkColorIndex);
        super.onSaveInstanceState(state);
    }

    private View createLayout() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.rgb(232, 231, 227));

        chrome = new ReaderChrome(this, new ReaderChrome.Listener() {
            @Override public void onOpenRequested() { launchOpenPicker(); }
            @Override public void onDefaultRequested() { launchDefaultSetup(); }
            @Override public void onToolSelected(AnnotationStore.Tool tool) { selectTool(tool); }
            @Override public void onColorRequested() { cycleInkColor(); }
            @Override public void onUndoRequested() { undoEdit(); }
            @Override public void onRedoRequested() { redoEdit(); }
            @Override public void onFitRequested() { zoomToFit(); }
            @Override public void onSaveRequested() { launchSavePicker(); }
        });
        root.addView(chrome, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        content = new FrameLayout(this);
        content.setBackgroundColor(Color.rgb(218, 219, 216));
        root.addView(content, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1));
        installSystemInsets(root);
        return root;
    }

    @SuppressWarnings("deprecation")
    private void installSystemInsets(View root) {
        root.setOnApplyWindowInsetsListener((view, windowInsets) -> {
            int left;
            int top;
            int right;
            int bottom;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                android.graphics.Insets safe = windowInsets.getInsets(
                        WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                left = safe.left;
                top = safe.top;
                right = safe.right;
                bottom = safe.bottom;
            } else {
                left = windowInsets.getSystemWindowInsetLeft();
                top = windowInsets.getSystemWindowInsetTop();
                right = windowInsets.getSystemWindowInsetRight();
                bottom = windowInsets.getSystemWindowInsetBottom();
            }
            top = Math.max(top, dp(32));
            chrome.setSystemInsets(left, top, right);
            content.setPadding(left, 0, right, 0);
            view.setPadding(0, 0, 0, bottom);
            return windowInsets;
        });
        root.requestApplyInsets();
    }

    private void selectTool(AnnotationStore.Tool selected) {
        if (pdfPageList == null) return;
        pdfPageList.setTool(selected);
        chrome.setSelectedTool(selected);
        int message;
        switch (selected) {
            case PEN: message = R.string.pen_help; break;
            case HIGHLIGHT: message = R.string.highlight_help; break;
            case SIGNATURE: message = R.string.signature_help; break;
            case ERASER: message = R.string.eraser_help; break;
            default: message = R.string.pan_help;
        }
        chrome.setStatus(getString(message));
    }

    private void cycleInkColor() {
        if (pdfPageList == null) return;
        inkColorIndex = (inkColorIndex + 1) % INK_COLORS.length;
        pdfPageList.setInkColor(INK_COLORS[inkColorIndex]);
        chrome.setInkColor(INK_COLORS[inkColorIndex], getColorName(inkColorIndex));
    }

    private String getColorName(int index) {
        switch (index) {
            case 1: return getString(R.string.color_red);
            case 2: return getString(R.string.color_blue);
            case 3: return getString(R.string.color_black);
            default: return getString(R.string.color_navy);
        }
    }

    private void undoEdit() {
        if (pdfPageList != null && pdfPageList.undoEdit()) {
            chrome.setStatus(getString(R.string.undo_complete));
        }
    }

    private void redoEdit() {
        if (pdfPageList != null && pdfPageList.redoEdit()) {
            chrome.setStatus(getString(R.string.redo_complete));
        }
    }

    private void zoomToFit() {
        if (pdfPageList != null) {
            pdfPageList.zoomToFit();
            selectTool(AnnotationStore.Tool.PAN);
        }
    }

    private void launchOpenPicker() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[]{"application/pdf", DOCX_MIME});
        startActivityForResult(intent, OPEN_DOCUMENT_REQUEST);
    }

    private void launchSavePicker() {
        if (pdfPageList == null) return;
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("application/pdf");
        intent.putExtra(Intent.EXTRA_TITLE, markedCopyName(currentDocumentName));
        startActivityForResult(intent, SAVE_DOCUMENT_REQUEST);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
        if (requestCode == SAVE_DOCUMENT_REQUEST) {
            saveAnnotatedCopy(uri);
            return;
        }
        if (requestCode != OPEN_DOCUMENT_REQUEST) return;
        if ((data.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION) != 0) {
            try {
                getContentResolver().takePersistableUriPermission(
                        uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } catch (SecurityException ignored) {
                // Some providers intentionally grant access only to this activity.
            }
        }
        openDocument(uri, getContentResolver().getType(uri));
    }

    private void saveAnnotatedCopy(Uri destination) {
        PdfPageList pages = pdfPageList;
        if (pages == null) return;
        chrome.setDocumentEnabled(false);
        chrome.setStatus(getString(R.string.saving_document));
        pages.saveAnnotated(destination, new PdfPageList.SaveListener() {
            @Override public void onSaved() {
                if (pdfPageList != pages) return;
                chrome.setDocumentEnabled(true);
                pages.refreshEditState();
                chrome.setStatus(getString(R.string.saved_document, displayName(destination)));
            }

            @Override public void onError(Exception error) {
                if (pdfPageList != pages) return;
                chrome.setDocumentEnabled(true);
                pages.refreshEditState();
                chrome.setStatus(getString(R.string.save_error, safeMessage(error)));
            }
        });
    }

    private boolean handleViewIntent(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction())
                || intent.getData() == null) {
            return false;
        }
        if (isSetupIntent(intent)) {
            handleDefaultSetupReturn();
            return true;
        }
        openDocument(intent.getData(), intent.getType());
        return true;
    }

    private boolean isSetupIntent(Intent intent) {
        Uri uri = intent == null ? null : intent.getData();
        return uri != null && SetupPdfProvider.AUTHORITY.equals(uri.getAuthority());
    }

    private void openDocument(Uri uri, String mimeType) {
        int generation = documentGeneration.incrementAndGet();
        currentDocumentUri = uri;
        currentDocumentMime = mimeType;
        currentDocumentName = displayName(uri);
        chrome.setDocumentTitle(currentDocumentName);
        chrome.setStatus(getString(R.string.opening_document, currentDocumentName));
        closePdfPageList();
        content.removeAllViews();
        String lower = currentDocumentName.toLowerCase(Locale.ROOT);
        if (DOCX_MIME.equals(mimeType) || lower.endsWith(".docx")) {
            openDocx(uri, generation);
        } else if ("application/pdf".equals(mimeType) || lower.endsWith(".pdf")) {
            openPdf(uri, generation);
        } else {
            chrome.setStatus(getString(R.string.unsupported_document));
        }
    }

    private void openDocx(Uri uri, int generation) {
        chrome.setDocumentEnabled(false);
        chrome.setToolsExpanded(false);
        documentExecutor.execute(() -> {
            try (InputStream input = getContentResolver().openInputStream(uri)) {
                if (input == null) {
                    throw new IllegalStateException("The document provider returned no data.");
                }
                String text = DocxReader.read(input);
                runOnUiThread(() -> {
                    if (documentGeneration.get() != generation) return;
                    DocxParagraphList document = new DocxParagraphList(this, text);
                    content.addView(document);
                    chrome.setStatus(getString(
                            R.string.docx_status, document.paragraphCount(), text.length()));
                });
            } catch (Exception error) {
                showError(generation, error);
            }
        });
    }

    private void openPdf(Uri uri, int generation) {
        PdfPageList pages = new PdfPageList(this, getContentResolver());
        pdfPageList = pages;
        pages.setInkColor(INK_COLORS[inkColorIndex]);
        pages.setEditListener(new PdfPageList.EditListener() {
            @Override
            public void onEditStateChanged(boolean canUndo, boolean canRedo, int markCount) {
                if (pdfPageList == pages) chrome.setEditState(canUndo, canRedo, markCount);
            }

            @Override
            public void onZoomChanged(float zoom) {
                if (pdfPageList == pages) chrome.setZoom(zoom);
            }

            @Override
            public void onChromeToggleRequested() {
                if (pdfPageList == pages) chrome.toggleCollapsed();
            }
        });
        content.addView(pages, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        pages.open(uri, new PdfPageList.Listener() {
            @Override public void onReady(int pageCount) {
                if (documentGeneration.get() != generation || pdfPageList != pages) return;
                chrome.setDocumentEnabled(true);
                pages.refreshEditState();
                chrome.setSelectedTool(AnnotationStore.Tool.PAN);
                String pageSummary = getResources().getQuantityString(
                        R.plurals.pdf_status, pageCount, pageCount);
                chrome.setStatus(getString(R.string.pdf_ready_status, pageSummary));
            }

            @Override public void onError(Exception error) {
                if (documentGeneration.get() == generation && pdfPageList == pages) {
                    chrome.setDocumentEnabled(false);
                    chrome.setStatus(getString(
                            R.string.document_error, safeMessage(error)));
                }
            }
        });
    }

    private void closePdfPageList() {
        chrome.setDocumentEnabled(false);
        if (pdfPageList != null) {
            pdfPageList.close();
            pdfPageList = null;
        }
    }

    private void launchDefaultSetup() {
        DefaultResolution resolution = resolveDefaultPdf();
        if (resolution.state == DefaultPdfChoice.State.LAWPDF) {
            chrome.setPdfDefault(true);
            chrome.setStatus(getString(R.string.already_pdf_default));
            Toast.makeText(this, R.string.already_pdf_default, Toast.LENGTH_SHORT).show();
            return;
        }
        if (resolution.state == DefaultPdfChoice.State.CHOOSER) {
            openPdfResolver();
            return;
        }

        CharSequence label = resolution.info == null || resolution.info.activityInfo == null
                ? getString(R.string.another_pdf_app)
                : resolution.info.loadLabel(getPackageManager());
        new AlertDialog.Builder(this)
                .setTitle(R.string.switch_pdf_default_title)
                .setMessage(getString(R.string.switch_pdf_default_message, label))
                .setNegativeButton(android.R.string.cancel, null)
                .setPositiveButton(R.string.open_app_settings, (dialog, which) ->
                        openCurrentHandlerSettings(resolution.packageName))
                .show();
    }

    private void openCurrentHandlerSettings(String packageName) {
        waitingForDefaultSettings = true;
        leftForDefaultSettings = false;
        Intent intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
        intent.setData(Uri.parse("package:" + packageName));
        try {
            startActivity(intent);
        } catch (ActivityNotFoundException error) {
            waitingForDefaultSettings = false;
            try {
                Intent fallback = Build.VERSION.SDK_INT >= Build.VERSION_CODES.N
                        ? new Intent(Settings.ACTION_MANAGE_DEFAULT_APPS_SETTINGS)
                        : new Intent(Settings.ACTION_SETTINGS);
                startActivity(fallback);
            } catch (ActivityNotFoundException ignored) {
                chrome.setStatus(getString(R.string.default_settings_unavailable));
            }
        }
    }

    private void continueDefaultSetupAfterSettings() {
        DefaultResolution resolution = resolveDefaultPdf();
        if (resolution.state == DefaultPdfChoice.State.LAWPDF) {
            showDefaultSuccess();
        } else if (resolution.state == DefaultPdfChoice.State.CHOOSER) {
            openPdfResolver();
        } else {
            chrome.setStatus(getString(R.string.clear_default_then_retry));
        }
    }

    private void openPdfResolver() {
        Intent intent = defaultPdfIntent();
        try {
            startActivity(intent);
            chrome.setStatus(getString(R.string.choose_lawpdf_always));
        } catch (ActivityNotFoundException error) {
            chrome.setStatus(getString(R.string.default_settings_unavailable));
        }
    }

    private Intent defaultPdfIntent() {
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.addCategory(Intent.CATEGORY_DEFAULT);
        intent.setDataAndType(SetupPdfProvider.SETUP_URI, "application/pdf");
        intent.setClipData(ClipData.newRawUri(
                getString(R.string.default_setup_document), SetupPdfProvider.SETUP_URI));
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        return intent;
    }

    private DefaultResolution resolveDefaultPdf() {
        Intent intent = defaultPdfIntent();
        ResolveInfo info = getPackageManager().resolveActivity(
                intent, android.content.pm.PackageManager.MATCH_DEFAULT_ONLY);
        String packageName = info == null || info.activityInfo == null
                ? null : info.activityInfo.packageName;
        String className = info == null || info.activityInfo == null
                ? null : info.activityInfo.name;
        return new DefaultResolution(
                DefaultPdfChoice.classify(getPackageName(), packageName, className),
                packageName,
                info);
    }

    private void updateDefaultState() {
        if (chrome != null) {
            chrome.setPdfDefault(resolveDefaultPdf().state == DefaultPdfChoice.State.LAWPDF);
        }
    }

    private void handleDefaultSetupReturn() {
        chrome.setStatus(getString(R.string.checking_pdf_default));
        chrome.postDelayed(() -> {
            if (resolveDefaultPdf().state == DefaultPdfChoice.State.LAWPDF) {
                showDefaultSuccess();
            } else {
                chrome.setPdfDefault(false);
                chrome.setStatus(getString(R.string.choose_always_next_time));
            }
        }, 350);
    }

    private void showDefaultSuccess() {
        chrome.setPdfDefault(true);
        chrome.setStatus(getString(R.string.pdf_default_success));
        Toast.makeText(this, R.string.pdf_default_success, Toast.LENGTH_SHORT).show();
    }

    private String displayName(Uri uri) {
        if ("content".equals(uri.getScheme())) {
            try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    if (column >= 0) {
                        String value = cursor.getString(column);
                        if (value != null && !value.trim().isEmpty()) return value;
                    }
                }
            } catch (RuntimeException ignored) {
                // Fall through to the URI segment.
            }
        }
        String segment = uri.getLastPathSegment();
        return segment == null || segment.isEmpty() ? "Document" : segment;
    }

    private void showError(int generation, Exception error) {
        runOnUiThread(() -> {
            if (documentGeneration.get() == generation) {
                chrome.setStatus(getString(R.string.document_error, safeMessage(error)));
            }
        });
    }

    private static String markedCopyName(String source) {
        String name = source == null || source.trim().isEmpty() ? "Document.pdf" : source;
        int dot = name.toLowerCase(Locale.ROOT).lastIndexOf(".pdf");
        String stem = dot > 0 ? name.substring(0, dot) : name;
        return stem + " - marked.pdf";
    }

    private static String safeMessage(Throwable error) {
        String message = error == null ? null : error.getMessage();
        return message == null || message.trim().isEmpty()
                ? (error == null ? "Unknown error" : error.getClass().getSimpleName())
                : message;
    }

    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        if (pdfPageList != null) pdfPageList.trimMemory(level);
    }

    @Override
    protected void onDestroy() {
        documentGeneration.incrementAndGet();
        closePdfPageList();
        documentExecutor.shutdownNow();
        super.onDestroy();
    }

    private static int dp(int value) {
        return Math.round(value
                * android.content.res.Resources.getSystem().getDisplayMetrics().density);
    }

    private static final class DefaultResolution {
        final DefaultPdfChoice.State state;
        final String packageName;
        final ResolveInfo info;

        DefaultResolution(
                DefaultPdfChoice.State state, String packageName, ResolveInfo info) {
            this.state = state;
            this.packageName = packageName;
            this.info = info;
        }
    }
}
