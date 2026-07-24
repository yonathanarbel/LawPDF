package com.lawpdf.mobile;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

public final class DefaultPdfChoiceTest {
    @Test
    public void recognizesLawPdfAsCurrentHandler() {
        assertEquals(
                DefaultPdfChoice.State.LAWPDF,
                DefaultPdfChoice.classify(
                        "com.lawpdf.mobile", "com.lawpdf.mobile", "com.lawpdf.mobile.MainActivity"));
    }

    @Test
    public void recognizesAndroidResolverAsChooser() {
        assertEquals(
                DefaultPdfChoice.State.CHOOSER,
                DefaultPdfChoice.classify(
                        "com.lawpdf.mobile", "android", "com.android.internal.app.ResolverActivity"));
        assertEquals(
                DefaultPdfChoice.State.CHOOSER,
                DefaultPdfChoice.classify("com.lawpdf.mobile", null, null));
    }

    @Test
    public void recognizesAnotherCurrentHandler() {
        assertEquals(
                DefaultPdfChoice.State.OTHER_APP,
                DefaultPdfChoice.classify(
                        "com.lawpdf.mobile", "com.adobe.reader", "com.adobe.reader.AdobeReader"));
    }
}
