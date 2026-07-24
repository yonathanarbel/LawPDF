package com.lawpdf.mobile;

/** Pure zoom and render-bucket decisions shared by the reader and unit tests. */
final class DisplayRenderPolicy {
    private DisplayRenderPolicy() {}

    static float clampZoom(float requested, float minimum, float maximum) {
        if (!Float.isFinite(requested)) return minimum;
        return Math.max(minimum, Math.min(maximum, requested));
    }

    static int renderBucket(int viewportWidth, float zoom, int bucketSize, int maximumWidth) {
        int safeViewport = Math.max(1, viewportWidth);
        int safeBucket = Math.max(1, bucketSize);
        int desired = Math.max(safeBucket, Math.round(safeViewport * zoom));
        int bucket = ((desired + safeBucket - 1) / safeBucket) * safeBucket;
        return Math.min(Math.max(safeBucket, maximumWidth), bucket);
    }

    static boolean shouldRequestSharpBitmap(boolean gestureInProgress, boolean exactBitmapCached) {
        return !gestureInProgress && !exactBitmapCached;
    }
}
