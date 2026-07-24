# Android reader UX research

This note records the source-backed choices behind LawPDF 0.2.0. It is intentionally narrower than a feature wishlist: the goal is a dependable phone reader and marking surface without introducing a large, unstable dependency.

## Rendering and gestures

- Android documents `PdfRenderer.Page.render` as a long-running operation and supports a transform matrix and clip region. Rendering therefore remains serialized off the main thread, while the UI reuses the last valid bitmap during a gesture: <https://developer.android.com/reference/android/graphics/pdf/PdfRenderer.Page>
- Android's gesture guidance uses `ScaleGestureDetector` and its focal point/span data for continuous pinch scaling. LawPDF follows that platform path and adds double-tap as an accessible one-finger shortcut: <https://developer.android.com/develop/ui/views/touch-and-input/gestures/scale>
- Android's rendering guidance warns against expensive bitmap work on the UI thread and recommends animation-frame invalidation for visual property updates. LawPDF coalesces zoom layout work with `postOnAnimation` and performs PDF rasterization on its renderer worker: <https://developer.android.com/topic/performance/vitals/render>

The key visual rule is continuity: a lower-resolution page is preferable to a white page. During pinch, the current bitmap scales immediately. After the fingers lift, one exact bucket is rendered and atomically replaces it.

## App bar and editing controls

- Material app bars define pinned, enter-always, and exit-until-collapsed behaviors. LawPDF uses the same large-to-small hierarchy but keeps collapse explicit through both the page surface and a 48 dp arrow, preventing an annotation gesture from unexpectedly moving the controls: <https://developer.android.com/develop/ui/compose/components/app-bars>
- Adobe Acrobat Mobile uses a tap on the document to enter or leave full-screen reading. LawPDF adopts that discoverable reader convention while preserving a compact **Open** action: <https://helpx.adobe.com/acrobat/mobile/view-manage-files/viewing-modes.html>
- Xodo separates view, annotate, draw, and fill/sign functions and places annotation tools in a horizontally scrollable toolbar. LawPDF similarly separates the app bar from a collapsible horizontal editing palette: <https://feedback.xodo.com/support/solutions/articles/35000199773-getting-started-xodo-basics>
- Android accessibility guidance calls for touch targets of at least 48 dp and meaningful content descriptions. Every interactive chrome item meets that size, disabled actions remain visually distinct, and dynamic status/zoom/default state is announced: <https://developer.android.com/guide/topics/ui/accessibility/views/apps-views>

## Default PDF behavior

Android owns preferred-app assignment; an app cannot silently appoint itself as the PDF default. There is no PDF role in `RoleManager`, and deprecated preferred-activity APIs do not provide a legitimate bypass:

- Intent resolution and the system chooser: <https://developer.android.com/guide/components/intents-filters>
- Package-manager preferred activity constraints: <https://developer.android.com/reference/android/content/pm/PackageManager>
- Available system roles: <https://developer.android.com/reference/android/app/role/RoleManager>
- App-details settings action: <https://developer.android.com/reference/android/provider/Settings>
- Package visibility for querying matching PDF handlers: <https://developer.android.com/training/package-visibility/declaring>

LawPDF's **Make default** button therefore minimizes the platform-required path: it tests a generated private setup PDF, opens the system resolver directly when no default exists, or explains and opens the current handler's app-details screen when its default must first be cleared.

## Jetpack PDF decision

The official AndroidX PDF viewer now demonstrates useful architecture: progressive layout, visible-page rendering, and bitmap release for off-screen pages. Its editable fragment also adds platform annotations and form filling on sufficiently new Android extension levels:

- Viewer guide: <https://developer.android.com/develop/ui/views/layout/pdf/pdf-viewer>
- Fragment reference: <https://developer.android.com/reference/androidx/pdf/viewer/fragment/PdfViewerFragment>
- Release history: <https://developer.android.com/jetpack/androidx/releases/pdf>

LawPDF 0.2.0 borrows the visible-page/two-stage rendering ideas but does not migrate to the current alpha editing stack. The alpha dependency has narrower platform requirements and a documented large-file performance limitation; retaining the framework renderer preserves the app's API 23 support and small dependency surface while the new annotation API matures.
