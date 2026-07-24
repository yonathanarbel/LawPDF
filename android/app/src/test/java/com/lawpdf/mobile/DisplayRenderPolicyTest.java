package com.lawpdf.mobile;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class DisplayRenderPolicyTest {
    @Test
    public void zoomIsFiniteAndClamped() {
        assertEquals(1f, DisplayRenderPolicy.clampZoom(Float.NaN, 1f, 4f), 0f);
        assertEquals(1f, DisplayRenderPolicy.clampZoom(Float.POSITIVE_INFINITY, 1f, 4f), 0f);
        assertEquals(1f, DisplayRenderPolicy.clampZoom(0.25f, 1f, 4f), 0f);
        assertEquals(4f, DisplayRenderPolicy.clampZoom(8f, 1f, 4f), 0f);
        assertEquals(2.5f, DisplayRenderPolicy.clampZoom(2.5f, 1f, 4f), 0f);
    }

    @Test
    public void renderWidthUsesStableBucketsAndCap() {
        assertEquals(512, DisplayRenderPolicy.renderBucket(400, 1f, 128, 2560));
        assertEquals(896, DisplayRenderPolicy.renderBucket(400, 2.1f, 128, 2560));
        assertEquals(2560, DisplayRenderPolicy.renderBucket(1000, 4f, 128, 2560));
    }

    @Test
    public void sharpRenderWaitsForGestureToSettle() {
        assertFalse(DisplayRenderPolicy.shouldRequestSharpBitmap(true, false));
        assertFalse(DisplayRenderPolicy.shouldRequestSharpBitmap(false, true));
        assertTrue(DisplayRenderPolicy.shouldRequestSharpBitmap(false, false));
    }
}
