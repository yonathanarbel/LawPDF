package com.lawpdf.mobile;

import android.annotation.SuppressLint;
import android.content.ContentResolver;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.graphics.pdf.PdfDocument;
import android.graphics.pdf.PdfRenderer;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.util.LruCache;
import android.view.GestureDetector;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.widget.BaseAdapter;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.TextView;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Continuous, recycling PDF reader with pinch zoom, horizontal panning, freehand
 * markup, and flattened annotated-copy export. All PdfRenderer access is serialized.
 */
@SuppressLint({"ViewConstructor", "ClickableViewAccessibility"})
final class PdfPageList extends ListView implements AutoCloseable {
    interface Listener {
        void onReady(int pageCount);
        void onError(Exception error);
    }

    interface SaveListener {
        void onSaved();
        void onError(Exception error);
    }

    interface EditListener {
        void onEditStateChanged(boolean canUndo, boolean canRedo, int markCount);
        void onZoomChanged(float zoom);
        void onChromeToggleRequested();
    }

    private static final int WIDTH_BUCKET = 128;
    private static final int MAX_RENDER_WIDTH = 2560;
    private static final int MAX_EXPORT_WIDTH = 2560;
    private static final int MIN_CACHE_KIB = 16 * 1024;
    private static final int MAX_CACHE_KIB = 64 * 1024;
    private static final float MIN_ZOOM = 1f;
    private static final float MAX_ZOOM = 4f;

    private final Context context;
    private final ContentResolver resolver;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ThreadPoolExecutor worker = new ThreadPoolExecutor(
            1, 1, 10, TimeUnit.SECONDS, new LinkedBlockingQueue<>());
    private final AtomicInteger generation = new AtomicInteger();
    private final Set<CacheKey> pending = Collections.newSetFromMap(new ConcurrentHashMap<>());
    private final LruCache<CacheKey, Bitmap> cache;
    private final AnnotationStore annotations = new AnnotationStore();
    private final int touchSlop;
    private final GestureDetector gestureDetector;
    private final ScaleGestureDetector scaleGestureDetector;

    private ParcelFileDescriptor descriptor;
    private PdfRenderer renderer;
    private PageAdapter adapter;
    private EditListener editListener;
    private AnnotationStore.Tool tool = AnnotationStore.Tool.PAN;
    private int inkColor = Color.rgb(20, 45, 80);
    private float zoom = 1f;
    private float horizontalOffset;
    private float downX;
    private float downY;
    private float lastX;
    private boolean horizontalDragging;
    private boolean pinching;
    private boolean zoomLayoutPosted;
    private AnnotationStore.Stroke activeStroke;

    PdfPageList(Context context, ContentResolver resolver) {
        super(context);
        this.context = context;
        this.resolver = resolver;
        touchSlop = ViewConfiguration.get(context).getScaledTouchSlop();
        int memoryKib = (int) Math.min(Integer.MAX_VALUE, Runtime.getRuntime().maxMemory() / 1024L);
        int cacheKib = Math.max(MIN_CACHE_KIB, Math.min(MAX_CACHE_KIB, memoryKib / 8));
        cache = new LruCache<CacheKey, Bitmap>(cacheKib) {
            @Override protected int sizeOf(CacheKey key, Bitmap value) {
                return Math.max(1, value.getAllocationByteCount() / 1024);
            }
        };
        setBackgroundColor(Color.rgb(214, 213, 210));
        setDividerHeight(0);
        setClipToPadding(false);
        setPadding(0, dp(6), 0, dp(24));
        scaleGestureDetector = new ScaleGestureDetector(context,
                new ScaleGestureDetector.SimpleOnScaleGestureListener() {
                    @Override public boolean onScaleBegin(ScaleGestureDetector detector) {
                        if (activeStroke != null) {
                            activeStroke = null;
                            invalidateVisiblePages();
                        }
                        pinching = true;
                        return true;
                    }

                    @Override public boolean onScale(ScaleGestureDetector detector) {
                        setZoom(zoom * detector.getScaleFactor(), detector.getFocusX());
                        return true;
                    }

                    @Override public void onScaleEnd(ScaleGestureDetector detector) {
                        pinching = false;
                        settleZoom();
                    }
                });
        gestureDetector = new GestureDetector(context,
                new GestureDetector.SimpleOnGestureListener() {
                    @Override public boolean onDown(MotionEvent event) { return true; }

                    @Override public boolean onSingleTapConfirmed(MotionEvent event) {
                        if (tool == AnnotationStore.Tool.PAN && editListener != null) {
                            editListener.onChromeToggleRequested();
                            return true;
                        }
                        return false;
                    }

                    @Override public boolean onDoubleTap(MotionEvent event) {
                        if (tool != AnnotationStore.Tool.PAN) return false;
                        float next = zoom < 1.5f ? 2f : 1f;
                        setZoom(next, event.getX());
                        settleZoom();
                        return true;
                    }
                });
        worker.allowCoreThreadTimeOut(true);
    }

    void setEditListener(EditListener listener) {
        editListener = listener;
        notifyEditState();
        if (listener != null) listener.onZoomChanged(zoom);
    }

    void setTool(AnnotationStore.Tool selected) {
        tool = selected == null ? AnnotationStore.Tool.PAN : selected;
        activeStroke = null;
    }

    AnnotationStore.Tool getTool() { return tool; }

    void setInkColor(int color) { inkColor = color; }
    int getInkColor() { return inkColor; }
    float getZoom() { return zoom; }
    int markCount() { return annotations.size(); }
    void refreshEditState() { notifyEditState(); }

    void zoomToFit() { setZoom(1f, getWidth() * 0.5f); }

    boolean undoEdit() {
        boolean changed = annotations.undo();
        if (changed) {
            invalidateVisiblePages();
            notifyEditState();
        }
        return changed;
    }

    boolean redoEdit() {
        boolean changed = annotations.redo();
        if (changed) {
            invalidateVisiblePages();
            notifyEditState();
        }
        return changed;
    }

    void open(Uri uri, Listener listener) {
        int token = generation.incrementAndGet();
        worker.getQueue().clear();
        pending.clear();
        cache.evictAll();
        annotations.clear();
        zoom = 1f;
        horizontalOffset = 0f;
        notifyEditState();
        worker.execute(() -> {
            closeRenderer();
            try {
                descriptor = resolver.openFileDescriptor(uri, "r");
                if (descriptor == null) throw new IOException("The document provider returned no PDF data.");
                renderer = new PdfRenderer(descriptor);
                int pages = renderer.getPageCount();
                main.post(() -> {
                    if (generation.get() != token) return;
                    adapter = new PageAdapter(context, pages, token);
                    setAdapter(adapter);
                    listener.onReady(pages);
                });
            } catch (Exception error) {
                main.post(() -> {
                    if (generation.get() == token) listener.onError(error);
                });
            }
        });
    }

    void saveAnnotated(Uri destination, SaveListener listener) {
        int token = generation.get();
        List<AnnotationStore.Stroke> marks = annotations.snapshot();
        worker.execute(() -> {
            File temporary = null;
            try {
                if (renderer == null || generation.get() != token) {
                    throw new IOException("The PDF is no longer open.");
                }
                temporary = File.createTempFile("lawpdf-marked-", ".pdf", context.getCacheDir());
                writeFlattenedPdf(temporary, marks, token);
                if (generation.get() != token) throw new IOException("The PDF changed while saving.");
                try (InputStream input = new FileInputStream(temporary);
                     OutputStream output = resolver.openOutputStream(destination, "wt")) {
                    if (output == null) throw new IOException("The document provider refused the save.");
                    byte[] buffer = new byte[64 * 1024];
                    int count;
                    while ((count = input.read(buffer)) >= 0) output.write(buffer, 0, count);
                }
                main.post(listener::onSaved);
            } catch (Exception error) {
                main.post(() -> listener.onError(error));
            } finally {
                if (temporary != null && !temporary.delete()) temporary.deleteOnExit();
            }
        });
    }

    private void writeFlattenedPdf(
            File destination, List<AnnotationStore.Stroke> marks, int token) throws IOException {
        PdfDocument output = new PdfDocument();
        try {
            int pageCount = renderer.getPageCount();
            Paint bitmapPaint = new Paint(Paint.ANTI_ALIAS_FLAG | Paint.FILTER_BITMAP_FLAG);
            for (int pageIndex = 0; pageIndex < pageCount; pageIndex++) {
                if (generation.get() != token) throw new IOException("Save cancelled.");
                try (PdfRenderer.Page source = renderer.openPage(pageIndex)) {
                    int pageWidth = Math.max(1, source.getWidth());
                    int pageHeight = Math.max(1, source.getHeight());
                    int renderWidth = Math.min(MAX_EXPORT_WIDTH, Math.max(pageWidth, pageWidth * 2));
                    int renderHeight = Math.max(1,
                            Math.round(renderWidth * pageHeight / (float) pageWidth));
                    Bitmap bitmap = Bitmap.createBitmap(renderWidth, renderHeight, Bitmap.Config.ARGB_8888);
                    bitmap.eraseColor(Color.WHITE);
                    source.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_PRINT);
                    PdfDocument.PageInfo info = new PdfDocument.PageInfo.Builder(
                            pageWidth, pageHeight, pageIndex + 1).create();
                    PdfDocument.Page page = output.startPage(info);
                    Canvas canvas = page.getCanvas();
                    canvas.drawColor(Color.WHITE);
                    canvas.drawBitmap(bitmap, null,
                            new RectF(0, 0, pageWidth, pageHeight), bitmapPaint);
                    bitmap.recycle();
                    drawMarks(canvas, marks, pageIndex, 0f, 0f, pageWidth, pageHeight);
                    output.finishPage(page);
                }
            }
            try (OutputStream stream = new FileOutputStream(destination)) {
                output.writeTo(stream);
            }
        } finally {
            output.close();
        }
    }

    void trimMemory(int level) {
        if (level >= android.content.ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) {
            cache.trimToSize(Math.max(1, cache.maxSize() / 2));
        }
        if (level >= android.content.ComponentCallbacks2.TRIM_MEMORY_BACKGROUND) cache.evictAll();
    }

    @Override
    public void close() {
        generation.incrementAndGet();
        worker.getQueue().clear();
        pending.clear();
        cache.evictAll();
        annotations.clear();
        setAdapter(null);
        adapter = null;
        worker.execute(this::closeRenderer);
        worker.shutdown();
    }

    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        scaleGestureDetector.onTouchEvent(event);
        if (tool == AnnotationStore.Tool.PAN && !scaleGestureDetector.isInProgress()) {
            gestureDetector.onTouchEvent(event);
        }
        return super.dispatchTouchEvent(event);
    }

    @Override
    public boolean onInterceptTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN) {
            downX = lastX = event.getX();
            downY = event.getY();
            horizontalDragging = false;
            if (tool != AnnotationStore.Tool.PAN) return true;
            return super.onInterceptTouchEvent(event);
        }
        if (scaleGestureDetector.isInProgress() || event.getPointerCount() >= 2) return true;
        if (tool != AnnotationStore.Tool.PAN) return true;
        if (action == MotionEvent.ACTION_MOVE && zoom > MIN_ZOOM) {
            float dx = event.getX() - downX;
            float dy = event.getY() - downY;
            if (Math.abs(dx) > touchSlop && Math.abs(dx) > Math.abs(dy)) {
                horizontalDragging = true;
                lastX = event.getX();
                return true;
            }
        }
        return super.onInterceptTouchEvent(event);
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        if (scaleGestureDetector.isInProgress() || event.getPointerCount() >= 2 || pinching) return true;
        if (tool != AnnotationStore.Tool.PAN) return handleEditorTouch(event);
        if (horizontalDragging) {
            if (action == MotionEvent.ACTION_MOVE) {
                float x = event.getX();
                setHorizontalOffset(horizontalOffset + lastX - x);
                lastX = x;
            } else if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
                horizontalDragging = false;
            }
            return true;
        }
        return super.onTouchEvent(event);
    }

    private boolean handleEditorTouch(MotionEvent event) {
        int action = event.getActionMasked();
        PageHit hit = hitAt(event.getX(), event.getY());
        if (action == MotionEvent.ACTION_DOWN) {
            if (hit == null) return true;
            if (tool == AnnotationStore.Tool.ERASER) {
                if (annotations.eraseAt(hit.page, hit.x, hit.y, 0.028f)) {
                    invalidateVisiblePages();
                    notifyEditState();
                }
            } else {
                activeStroke = annotations.startStroke(
                        hit.page, tool, colorForTool(), widthForTool(), hit.x, hit.y);
                invalidateVisiblePages();
            }
            return true;
        }
        if (action == MotionEvent.ACTION_MOVE) {
            if (hit == null) return true;
            if (tool == AnnotationStore.Tool.ERASER) {
                if (annotations.eraseAt(hit.page, hit.x, hit.y, 0.028f)) {
                    invalidateVisiblePages();
                    notifyEditState();
                }
            } else if (activeStroke != null && activeStroke.page == hit.page) {
                activeStroke.add(hit.x, hit.y);
                invalidateVisiblePages();
            }
            return true;
        }
        if (action == MotionEvent.ACTION_UP) {
            if (activeStroke != null) {
                if (hit != null && activeStroke.page == hit.page) activeStroke.add(hit.x, hit.y);
                annotations.commit(activeStroke);
                activeStroke = null;
                invalidateVisiblePages();
                notifyEditState();
            }
            performClick();
            return true;
        }
        if (action == MotionEvent.ACTION_CANCEL) {
            activeStroke = null;
            invalidateVisiblePages();
            return true;
        }
        return true;
    }

    @Override public boolean performClick() {
        super.performClick();
        return true;
    }

    private int colorForTool() {
        if (tool == AnnotationStore.Tool.HIGHLIGHT) return Color.argb(90, 255, 214, 0);
        if (tool == AnnotationStore.Tool.SIGNATURE) return Color.rgb(20, 45, 80);
        return inkColor;
    }

    private float widthForTool() {
        if (tool == AnnotationStore.Tool.HIGHLIGHT) return 0.022f;
        if (tool == AnnotationStore.Tool.SIGNATURE) return 0.0045f;
        return 0.0035f;
    }

    private void setZoom(float requested, float focusX) {
        float next = DisplayRenderPolicy.clampZoom(requested, MIN_ZOOM, MAX_ZOOM);
        if (Math.abs(next - zoom) < 0.005f) return;
        float documentX = (horizontalOffset + focusX) / zoom;
        zoom = next;
        horizontalOffset = documentX * zoom - focusX;
        horizontalOffset = clampedOffset(horizontalOffset);
        scheduleZoomLayout();
        if (editListener != null) editListener.onZoomChanged(zoom);
    }

    private void scheduleZoomLayout() {
        if (zoomLayoutPosted) return;
        zoomLayoutPosted = true;
        int first = getFirstVisiblePosition();
        View firstView = getChildAt(0);
        int top = firstView == null ? 0 : firstView.getTop();
        postOnAnimation(() -> {
            zoomLayoutPosted = false;
            if (adapter != null) adapter.notifyDataSetChanged();
            if (firstView != null) setSelectionFromTop(first, top);
            invalidateVisiblePagesOnAnimation();
        });
    }

    private void settleZoom() {
        postOnAnimation(() -> {
            if (adapter != null) adapter.notifyDataSetChanged();
            requestSharpVisiblePages();
        });
    }

    private void setHorizontalOffset(float requested) {
        horizontalOffset = clampedOffset(requested);
        for (int index = 0; index < getChildCount(); index++) {
            View view = getChildAt(index);
            if (view instanceof PageRow) ((PageRow) view).surface.postInvalidateOnAnimation();
        }
    }

    private float clampedOffset(float requested) {
        int viewport = Math.max(1, getWidth() - getPaddingLeft() - getPaddingRight() - dp(16));
        float maximum = Math.max(0f, viewport * zoom - viewport);
        return Math.max(0f, Math.min(maximum, requested));
    }

    private PageHit hitAt(float x, float y) {
        int position = pointToPosition(Math.round(x), Math.round(y));
        if (position == INVALID_POSITION) return null;
        int childIndex = position - getFirstVisiblePosition();
        View child = getChildAt(childIndex);
        if (!(child instanceof PageRow)) return null;
        PageRow row = (PageRow) child;
        float surfaceTop = row.getTop() + row.surface.getTop();
        float localY = y - surfaceTop;
        if (localY < 0 || localY > row.surface.getHeight()) return null;
        float contentWidth = row.contentWidth();
        float contentHeight = row.surface.getHeight();
        float normalizedX = (x + horizontalOffset) / Math.max(1f, contentWidth);
        float normalizedY = localY / Math.max(1f, contentHeight);
        if (normalizedX < 0f || normalizedX > 1f || normalizedY < 0f || normalizedY > 1f) {
            return null;
        }
        return new PageHit(position, normalizedX, normalizedY);
    }

    private void requestPage(int pageIndex, int width, PageRow target, int token, boolean prefetch) {
        int renderWidth = Math.min(MAX_RENDER_WIDTH,
                Math.max(WIDTH_BUCKET, ((width + WIDTH_BUCKET - 1) / WIDTH_BUCKET) * WIDTH_BUCKET));
        CacheKey key = new CacheKey(pageIndex, renderWidth, token);
        Bitmap cached = cache.get(key);
        if (cached != null) {
            if (target != null) target.showBitmap(key, cached);
            return;
        }
        if (!pending.add(key)) return;

        worker.execute(() -> {
            Bitmap bitmap = null;
            Exception failure = null;
            try {
                if (generation.get() != token || renderer == null) return;
                try (PdfRenderer.Page page = renderer.openPage(pageIndex)) {
                    int height = Math.max(1,
                            Math.round(renderWidth * page.getHeight() / (float) page.getWidth()));
                    bitmap = Bitmap.createBitmap(renderWidth, height, Bitmap.Config.ARGB_8888);
                    bitmap.eraseColor(Color.WHITE);
                    page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY);
                }
            } catch (Exception error) {
                failure = error;
            } finally {
                pending.remove(key);
            }

            Bitmap rendered = bitmap;
            Exception renderFailure = failure;
            main.post(() -> {
                if (generation.get() != token) return;
                if (rendered != null) {
                    cache.put(key, rendered);
                    showRenderedOnVisibleRows(key, rendered);
                    if (!prefetch && adapter != null) {
                        int baseWidth = Math.max(WIDTH_BUCKET, Math.round(width / zoom));
                        if (pageIndex > 0) requestPage(pageIndex - 1, baseWidth, null, token, true);
                        if (pageIndex + 1 < adapter.getCount()) {
                            requestPage(pageIndex + 1, baseWidth, null, token, true);
                        }
                    }
                } else if (target != null && renderFailure != null) {
                    target.showError(key, renderFailure.getMessage());
                }
            });
        });
    }

    private void closeRenderer() {
        if (renderer != null) {
            renderer.close();
            renderer = null;
        }
        if (descriptor != null) {
            try { descriptor.close(); } catch (IOException ignored) {}
            descriptor = null;
        }
    }

    private void notifyEditState() {
        if (editListener != null) {
            editListener.onEditStateChanged(
                    annotations.canUndo(), annotations.canRedo(), annotations.size());
        }
    }

    private void invalidateVisiblePages() {
        for (int index = 0; index < getChildCount(); index++) {
            View view = getChildAt(index);
            if (view instanceof PageRow) ((PageRow) view).surface.invalidate();
        }
    }

    private void invalidateVisiblePagesOnAnimation() {
        for (int index = 0; index < getChildCount(); index++) {
            View view = getChildAt(index);
            if (view instanceof PageRow) ((PageRow) view).surface.postInvalidateOnAnimation();
        }
    }

    private void requestSharpVisiblePages() {
        for (int index = 0; index < getChildCount(); index++) {
            View view = getChildAt(index);
            if (!(view instanceof PageRow)) continue;
            PageRow row = (PageRow) view;
            CacheKey key = row.boundKey;
            if (key != null && cache.get(key) == null) {
                requestPage(key.page, key.width, row, key.generation, false);
            }
        }
    }

    private void showRenderedOnVisibleRows(CacheKey key, Bitmap bitmap) {
        for (int index = 0; index < getChildCount(); index++) {
            View view = getChildAt(index);
            if (view instanceof PageRow) ((PageRow) view).showBitmap(key, bitmap);
        }
    }

    private Bitmap findClosestCachedBitmap(int page, int token, int width) {
        Bitmap closest = null;
        int closestDistance = Integer.MAX_VALUE;
        for (Map.Entry<CacheKey, Bitmap> entry : cache.snapshot().entrySet()) {
            CacheKey key = entry.getKey();
            Bitmap candidate = entry.getValue();
            if (key.page != page || key.generation != token
                    || candidate == null || candidate.isRecycled()) continue;
            int distance = Math.abs(key.width - width);
            if (distance < closestDistance) {
                closest = candidate;
                closestDistance = distance;
            }
        }
        return closest;
    }

    private final class PageAdapter extends BaseAdapter {
        private final Context context;
        private final int pageCount;
        private final int token;

        PageAdapter(Context context, int pageCount, int token) {
            this.context = context;
            this.pageCount = pageCount;
            this.token = token;
        }

        @Override public int getCount() { return pageCount; }
        @Override public Integer getItem(int position) { return position; }
        @Override public long getItemId(int position) { return position; }
        @Override public boolean hasStableIds() { return true; }

        @Override
        public View getView(int position, View recycled, ViewGroup parent) {
            PageRow row = recycled instanceof PageRow ? (PageRow) recycled : new PageRow(context);
            int available = getWidth() - getPaddingLeft() - getPaddingRight() - dp(16);
            if (available <= 0) available = getResources().getDisplayMetrics().widthPixels - dp(16);
            int desiredRenderWidth = Math.max(WIDTH_BUCKET, Math.round(available * zoom));
            int bucket = DisplayRenderPolicy.renderBucket(
                    available, zoom, WIDTH_BUCKET, MAX_RENDER_WIDTH);
            CacheKey key = new CacheKey(position, bucket, token);
            row.bind(position, key, available);
            Bitmap cached = cache.get(key);
            if (cached != null) row.showBitmap(key, cached);
            else {
                Bitmap fallback = findClosestCachedBitmap(position, token, bucket);
                if (fallback != null) row.showFallback(fallback);
                if (DisplayRenderPolicy.shouldRequestSharpBitmap(pinching, fallback != null)) {
                    requestPage(position, desiredRenderWidth, row, token, false);
                }
            }
            return row;
        }
    }

    private final class PageRow extends LinearLayout {
        private final TextView label;
        private final PageSurface surface;
        private CacheKey boundKey;
        private int baseWidth;
        private float aspect = 1.294f;

        PageRow(Context context) {
            super(context);
            setOrientation(VERTICAL);
            setGravity(Gravity.CENTER_HORIZONTAL);
            setPadding(dp(8), dp(4), dp(8), dp(6));
            label = new TextView(context);
            label.setTextColor(Color.rgb(65, 65, 65));
            label.setTextSize(12);
            label.setGravity(Gravity.CENTER);
            addView(label, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT));
            surface = new PageSurface(context);
            addView(surface, new LayoutParams(LayoutParams.MATCH_PARENT, 1));
        }

        void bind(int page, CacheKey key, int width) {
            boolean pageChanged = boundKey == null
                    || boundKey.page != page
                    || boundKey.generation != key.generation;
            boundKey = key;
            baseWidth = width;
            label.setText(getResources().getString(R.string.page_number, page + 1));
            surface.bind(page, key, pageChanged);
            surface.setContentDescription(
                    getResources().getString(R.string.pdf_page_description, page + 1));
            updateHeight();
        }

        void showFallback(Bitmap bitmap) {
            aspect = bitmap.getHeight() / (float) bitmap.getWidth();
            surface.showFallback(bitmap);
            updateHeight();
        }

        void showBitmap(CacheKey key, Bitmap bitmap) {
            if (!key.equals(boundKey)) return;
            aspect = bitmap.getHeight() / (float) bitmap.getWidth();
            surface.showBitmap(key, bitmap);
            updateHeight();
        }

        void showError(CacheKey key, String message) {
            if (!key.equals(boundKey)) return;
            String detail = message == null || message.isEmpty() ? "" : ": " + message;
            label.setText(getResources().getString(
                    R.string.page_render_error, label.getText(), detail));
        }

        float contentWidth() { return baseWidth * zoom; }

        private void updateHeight() {
            int height = Math.max(1, Math.round(contentWidth() * aspect));
            LayoutParams params = (LayoutParams) surface.getLayoutParams();
            if (params.height != height) {
                params.height = height;
                surface.setLayoutParams(params);
            }
            surface.invalidate();
        }
    }

    private final class PageSurface extends View {
        private int page;
        private CacheKey boundKey;
        private Bitmap bitmap;
        private final Paint bitmapPaint = new Paint(Paint.ANTI_ALIAS_FLAG | Paint.FILTER_BITMAP_FLAG);
        private final RectF bitmapDestination = new RectF();

        PageSurface(Context context) {
            super(context);
            setBackgroundColor(Color.WHITE);
        }

        void bind(int page, CacheKey key, boolean pageChanged) {
            this.page = page;
            boundKey = key;
            if (pageChanged) bitmap = null;
            postInvalidateOnAnimation();
        }

        void showBitmap(CacheKey key, Bitmap rendered) {
            if (!key.equals(boundKey)) return;
            bitmap = rendered;
            postInvalidateOnAnimation();
        }

        void showFallback(Bitmap rendered) {
            bitmap = rendered;
            postInvalidateOnAnimation();
        }

        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            float width = ((PageRow) getParent()).contentWidth();
            bitmapDestination.set(
                    -horizontalOffset, 0, -horizontalOffset + width, getHeight());
            if (bitmap != null && !bitmap.isRecycled()) {
                canvas.drawBitmap(bitmap, null, bitmapDestination, bitmapPaint);
            }
            List<AnnotationStore.Stroke> pageMarks = annotations.forPage(page);
            if (activeStroke != null && activeStroke.page == page) pageMarks.add(activeStroke);
            drawMarks(canvas, pageMarks, page,
                    -horizontalOffset, 0f, width, getHeight());
        }
    }

    private static void drawMarks(Canvas canvas, List<AnnotationStore.Stroke> marks,
                                  int page, float left, float top, float width, float height) {
        for (AnnotationStore.Stroke stroke : marks) {
            if (stroke.page != page || stroke.points.isEmpty()) continue;
            Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeCap(Paint.Cap.ROUND);
            paint.setStrokeJoin(Paint.Join.ROUND);
            paint.setColor(stroke.color);
            paint.setStrokeWidth(Math.max(1f, stroke.width * Math.min(width, height)));
            if (stroke.points.size() == 1) {
                AnnotationStore.Point point = stroke.points.get(0);
                canvas.drawPoint(left + point.x * width, top + point.y * height, paint);
                continue;
            }
            Path path = new Path();
            AnnotationStore.Point first = stroke.points.get(0);
            path.moveTo(left + first.x * width, top + first.y * height);
            for (int index = 1; index < stroke.points.size(); index++) {
                AnnotationStore.Point point = stroke.points.get(index);
                path.lineTo(left + point.x * width, top + point.y * height);
            }
            canvas.drawPath(path, paint);
        }
    }

    private static final class PageHit {
        final int page;
        final float x;
        final float y;

        PageHit(int page, float x, float y) {
            this.page = page;
            this.x = x;
            this.y = y;
        }
    }

    private static final class CacheKey {
        final int page;
        final int width;
        final int generation;

        CacheKey(int page, int width, int generation) {
            this.page = page;
            this.width = width;
            this.generation = generation;
        }

        @Override public boolean equals(Object other) {
            if (this == other) return true;
            if (!(other instanceof CacheKey)) return false;
            CacheKey key = (CacheKey) other;
            return page == key.page && width == key.width && generation == key.generation;
        }

        @Override public int hashCode() {
            int result = page;
            result = 31 * result + width;
            return 31 * result + generation;
        }
    }

    private static int dp(int value) {
        return Math.round(value * android.content.res.Resources.getSystem().getDisplayMetrics().density);
    }
}
