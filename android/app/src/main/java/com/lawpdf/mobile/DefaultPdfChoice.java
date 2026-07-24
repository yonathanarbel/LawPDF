package com.lawpdf.mobile;

/** Pure classification of Android's result for a generic PDF view intent. */
final class DefaultPdfChoice {
    enum State { LAWPDF, CHOOSER, OTHER_APP }

    private DefaultPdfChoice() {}

    static State classify(
            String ownPackage, String resolvedPackage, String resolvedClassName) {
        if (ownPackage != null && ownPackage.equals(resolvedPackage)) {
            return State.LAWPDF;
        }
        String packageName = resolvedPackage == null ? "" : resolvedPackage;
        String className = resolvedClassName == null ? "" : resolvedClassName;
        if (packageName.isEmpty()
                || "android".equals(packageName)
                || className.contains("ResolverActivity")
                || className.contains("ChooserActivity")) {
            return State.CHOOSER;
        }
        return State.OTHER_APP;
    }
}
