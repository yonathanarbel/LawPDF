# Windows printing

The Windows print-preview window is available in **0.2.46 and later**.
Earlier Windows versions do not include this print window.
The existing Mac and Linux printing features use their own platform dialogs.

## Open the print window

Open a PDF, then press **Ctrl+P**, click the toolbar's **Print** button, or choose
**Print…** from the **Export** menu. Printing uses the original PDF layout even
when you are reading in Review Mode.

The window shows a page preview beside the controls. Use the arrows below the
preview to inspect each prepared sheet. The footer shows the selected page count,
estimated physical sheets, and copies. Choose **Print** to send the prepared
sheets to the selected Windows printer.

## Print settings

| Setting | Options |
| --- | --- |
| Destination | Select an installed printer; refresh the list after adding one in Windows Settings. |
| Copies | 1–999, subject to the printer's limit. **Collate** groups pages into complete copies. |
| Pages | All pages, the current page, or a range such as `1–3, 5, 8–12`. Numbers refer to PDF page positions. |
| Order | Print only odd or even selected pages, or reverse the selected order. |
| Paper | Letter, Legal, A4, or A3; portrait or landscape. The printer must support the selected size. |
| Sides | One-sided, or automatic two-sided printing with long-edge or short-edge binding. A duplex-capable printer is required. |
| Pages per sheet | 1, 2, or 4 PDF pages. |
| Scale | Fit inside the selected margins, shrink oversized pages only, or actual size at 100%. Actual-size content that extends outside its allotted space is cropped. |
| Margins | Set the space around the page layout in millimeters. Hardware margins can still trim edge content. |
| Appearance | Black-and-white output, and whether to include annotations. |

Choose **System dialog…** for device-specific properties such as trays and print
quality. Set paper size and orientation in LawPDF so the preview matches the
printed sheets. If those settings change in the Windows dialog, LawPDF asks you
to update its layout before printing. Unsupported paper or duplex settings
produce an error rather than silently changing the job.

## Documents, preparation, and output

The print window takes a temporary copy of the current document and its current
annotations, including unsaved edits. Preparing and printing that copy does not
rewrite the original PDF. Turning **Include annotations** off removes markup
from the print copy; filled form fields remain. Close and reopen the print window
to take a fresh copy after changing the document.

Printing prepares image-based sheets at a target of **300 DPI**. Text and graphics
are rasterized rather than sent as PDF vectors. Individual page rasters are
capped at 16 million pixels and complete sheets at 32 million pixels, so unusually
large pages can have lower effective detail. The preview uses the same layout at
a lower screen resolution.

Large jobs can take time to prepare. **Cancel preparation** stops that stage;
once Windows receives the job, use the Windows print queue to cancel it. A
“sent to the printer” message confirms submission to Windows, not that paper has
finished printing. Check the queue and printer for offline, paper, or toner issues.
