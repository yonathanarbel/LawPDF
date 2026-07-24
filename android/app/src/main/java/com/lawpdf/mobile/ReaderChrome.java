package com.lawpdf.mobile;

import android.annotation.SuppressLint;
import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.HorizontalScrollView;
import android.widget.LinearLayout;
import android.widget.TextView;

import java.util.EnumMap;
import java.util.Map;

/**
 * Large-to-small reader chrome inspired by Material's collapsing app bars.
 * Primary actions remain obvious when expanded; a single compact row preserves
 * document space in focus mode.
 */
@SuppressLint("ViewConstructor")
final class ReaderChrome extends LinearLayout {
    interface Listener {
        void onOpenRequested();
        void onDefaultRequested();
        void onToolSelected(AnnotationStore.Tool tool);
        void onColorRequested();
        void onUndoRequested();
        void onRedoRequested();
        void onFitRequested();
        void onSaveRequested();
    }

    private static final int NAVY = Color.rgb(20, 45, 70);
    private static final int NAVY_RAISED = Color.rgb(29, 61, 91);
    private static final int PAPER = Color.rgb(248, 246, 240);
    private static final int PALE_BLUE = Color.rgb(230, 237, 243);
    private static final int INK = Color.rgb(30, 43, 54);
    private static final int MUTED = Color.rgb(91, 108, 122);
    private static final int BLUE = Color.rgb(54, 105, 162);
    private static final int GOLD = Color.rgb(235, 183, 72);
    private static final int GREEN = Color.rgb(54, 128, 92);
    private static final int WHITE = Color.WHITE;

    private final Listener listener;
    private final LinearLayout headerRow;
    private final TextView logo;
    private final TextView eyebrow;
    private final TextView title;
    private final TextView compactOpen;
    private final TextView collapseAction;
    private final LinearLayout quickActions;
    private final TextView openAction;
    private final TextView defaultAction;
    private final TextView editAction;
    private final TextView status;
    private final HorizontalScrollView toolsScroll;
    private final Map<AnnotationStore.Tool, TextView> toolActions =
            new EnumMap<>(AnnotationStore.Tool.class);
    private final TextView colorAction;
    private final TextView undoAction;
    private final TextView redoAction;
    private final TextView fitAction;
    private final TextView saveAction;
    private final TextView zoomLabel;
    private boolean collapsed;
    private boolean toolsExpanded;
    private boolean documentEnabled;
    private boolean pdfDefault;

    ReaderChrome(Context context, Listener listener) {
        super(context);
        this.listener = listener;
        setOrientation(VERTICAL);
        setBackgroundColor(NAVY);
        setElevation(dp(5));

        headerRow = new LinearLayout(context);
        headerRow.setGravity(Gravity.CENTER_VERTICAL);
        headerRow.setPadding(dp(12), dp(7), dp(8), dp(5));

        logo = text("L", 19, Typeface.BOLD, NAVY);
        logo.setGravity(Gravity.CENTER);
        logo.setBackground(roundRect(GOLD, GOLD, 12));
        logo.setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
        headerRow.addView(logo, sized(dp(40), dp(40)));

        LinearLayout titleStack = new LinearLayout(context);
        titleStack.setOrientation(VERTICAL);
        titleStack.setPadding(dp(11), 0, dp(8), 0);
        eyebrow = text("LAWPDF", 10, Typeface.BOLD, Color.rgb(188, 207, 225));
        eyebrow.setLetterSpacing(0.18f);
        title = text(getResources().getString(R.string.no_document_title),
                18, Typeface.BOLD, WHITE);
        title.setSingleLine(true);
        title.setEllipsize(TextUtils.TruncateAt.END);
        titleStack.addView(eyebrow);
        titleStack.addView(title);
        headerRow.addView(titleStack, weighted());

        compactOpen = action(getResources().getString(R.string.open_short), listener::onOpenRequested);
        compactOpen.setVisibility(GONE);
        styleAction(compactOpen, NAVY_RAISED, WHITE, NAVY_RAISED);
        headerRow.addView(compactOpen, wrapWithMargins(2));

        collapseAction = action("\u25B2", this::toggleCollapsed);
        collapseAction.setContentDescription(
                getResources().getString(R.string.collapse_top_bar));
        styleAction(collapseAction, NAVY_RAISED, WHITE, NAVY_RAISED);
        headerRow.addView(collapseAction, sizedWithMargins(dp(48), dp(48), 2));
        addView(headerRow, matchWrap());

        quickActions = new LinearLayout(context);
        quickActions.setGravity(Gravity.CENTER);
        quickActions.setPadding(dp(8), dp(2), dp(8), dp(7));

        openAction = action(
                getResources().getString(R.string.open_document), listener::onOpenRequested);
        styleAction(openAction, WHITE, INK, WHITE);
        addWeightedAction(quickActions, openAction);

        defaultAction = action(
                getResources().getString(R.string.make_pdf_default), listener::onDefaultRequested);
        styleAction(defaultAction, GOLD, NAVY, GOLD);
        addWeightedAction(quickActions, defaultAction);

        editAction = action(getResources().getString(R.string.edit_tools), this::toggleTools);
        styleAction(editAction, PALE_BLUE, INK, PALE_BLUE);
        addWeightedAction(quickActions, editAction);
        addView(quickActions, matchWrap());

        status = text(getResources().getString(R.string.empty_message), 13, Typeface.NORMAL,
                Color.rgb(219, 229, 238));
        status.setPadding(dp(14), dp(5), dp(14), dp(8));
        status.setMaxLines(2);
        status.setAccessibilityLiveRegion(ACCESSIBILITY_LIVE_REGION_POLITE);
        addView(status, matchWrap());

        toolsScroll = new HorizontalScrollView(context);
        toolsScroll.setHorizontalScrollBarEnabled(false);
        toolsScroll.setFillViewport(true);
        toolsScroll.setBackgroundColor(PAPER);
        toolsScroll.setVisibility(GONE);

        LinearLayout tools = new LinearLayout(context);
        tools.setGravity(Gravity.CENTER_VERTICAL);
        tools.setPadding(dp(8), dp(6), dp(8), dp(6));
        addTool(tools, AnnotationStore.Tool.PAN, R.string.tool_pan);
        addTool(tools, AnnotationStore.Tool.PEN, R.string.tool_pen);
        addTool(tools, AnnotationStore.Tool.HIGHLIGHT, R.string.tool_highlight);
        addTool(tools, AnnotationStore.Tool.SIGNATURE, R.string.tool_signature);
        addTool(tools, AnnotationStore.Tool.ERASER, R.string.tool_eraser);

        colorAction = toolAction(R.string.tool_color, listener::onColorRequested);
        tools.addView(colorAction, wrapWithMargins(3));
        undoAction = toolAction(R.string.tool_undo, listener::onUndoRequested);
        tools.addView(undoAction, wrapWithMargins(3));
        redoAction = toolAction(R.string.tool_redo, listener::onRedoRequested);
        tools.addView(redoAction, wrapWithMargins(3));
        fitAction = toolAction(R.string.tool_fit, listener::onFitRequested);
        tools.addView(fitAction, wrapWithMargins(3));
        saveAction = toolAction(R.string.tool_save_copy, listener::onSaveRequested);
        styleAction(saveAction, GOLD, NAVY, GOLD);
        tools.addView(saveAction, wrapWithMargins(3));
        zoomLabel = text("100%", 12, Typeface.BOLD, MUTED);
        zoomLabel.setGravity(Gravity.CENTER);
        zoomLabel.setMinWidth(dp(58));
        zoomLabel.setContentDescription(getResources().getString(R.string.current_zoom));
        tools.addView(zoomLabel, sizedWithMargins(dp(60), dp(48), 3));
        toolsScroll.addView(tools, new HorizontalScrollView.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        addView(toolsScroll, matchWrap());

        setDocumentEnabled(false);
        setSelectedTool(AnnotationStore.Tool.PAN);
        setPdfDefault(false);
    }

    void setSystemInsets(int left, int top, int right) {
        setPadding(left, top, right, 0);
    }

    void setDocumentTitle(String value) {
        title.setText(value == null || value.trim().isEmpty()
                ? getResources().getString(R.string.no_document_title)
                : value);
    }

    void setStatus(CharSequence message) {
        status.setText(message);
    }

    void setDocumentEnabled(boolean enabled) {
        documentEnabled = enabled;
        setEnabledState(editAction, enabled);
        for (TextView action : toolActions.values()) setEnabledState(action, enabled);
        setEnabledState(colorAction, enabled);
        setEnabledState(fitAction, enabled);
        setEnabledState(saveAction, enabled);
        if (!enabled) {
            toolsExpanded = false;
            setEnabledState(undoAction, false);
            setEnabledState(redoAction, false);
        }
        refreshVisibility();
    }

    void setSelectedTool(AnnotationStore.Tool selected) {
        for (Map.Entry<AnnotationStore.Tool, TextView> entry : toolActions.entrySet()) {
            boolean active = entry.getKey() == selected;
            styleAction(
                    entry.getValue(),
                    active ? BLUE : WHITE,
                    active ? WHITE : INK,
                    active ? BLUE : Color.rgb(215, 222, 227));
            entry.getValue().setSelected(active);
        }
    }

    void setInkColor(int color, String colorName) {
        colorAction.setText(getResources().getString(R.string.tool_color_dot));
        colorAction.setTextColor(color);
        colorAction.setContentDescription(
                getResources().getString(R.string.tool_color_description, colorName));
    }

    void setEditState(boolean canUndo, boolean canRedo, int markCount) {
        setEnabledState(undoAction, documentEnabled && canUndo);
        setEnabledState(redoAction, documentEnabled && canRedo);
        saveAction.setText(markCount == 0
                ? getResources().getString(R.string.tool_save_copy)
                : getResources().getString(R.string.tool_save_copy_count, markCount));
    }

    void setZoom(float zoom) {
        zoomLabel.setText(getResources().getString(
                R.string.zoom_percent, Math.round(zoom * 100f)));
        zoomLabel.setContentDescription(getResources().getString(
                R.string.current_zoom_percent, Math.round(zoom * 100f)));
    }

    void setPdfDefault(boolean isDefault) {
        pdfDefault = isDefault;
        defaultAction.setText(isDefault
                ? getResources().getString(R.string.pdf_default_active)
                : getResources().getString(R.string.make_pdf_default));
        defaultAction.setContentDescription(isDefault
                ? getResources().getString(R.string.pdf_default_active_description)
                : getResources().getString(R.string.make_pdf_default_description));
        styleAction(defaultAction, isDefault ? GREEN : GOLD, isDefault ? WHITE : NAVY,
                isDefault ? GREEN : GOLD);
    }

    boolean isCollapsed() {
        return collapsed;
    }

    boolean areToolsExpanded() {
        return toolsExpanded;
    }

    void setCollapsed(boolean value) {
        collapsed = value;
        refreshVisibility();
    }

    void setToolsExpanded(boolean value) {
        toolsExpanded = value && documentEnabled;
        if (toolsExpanded) collapsed = false;
        refreshVisibility();
    }

    void toggleCollapsed() {
        setCollapsed(!collapsed);
    }

    private void toggleTools() {
        setToolsExpanded(!toolsExpanded);
    }

    private void refreshVisibility() {
        logo.setVisibility(collapsed ? GONE : VISIBLE);
        eyebrow.setVisibility(collapsed ? GONE : VISIBLE);
        compactOpen.setVisibility(collapsed ? VISIBLE : GONE);
        quickActions.setVisibility(collapsed ? GONE : VISIBLE);
        status.setVisibility(collapsed ? GONE : VISIBLE);
        toolsScroll.setVisibility(
                !collapsed && toolsExpanded && documentEnabled ? VISIBLE : GONE);
        collapseAction.setText(collapsed ? "\u25BC" : "\u25B2");
        collapseAction.setContentDescription(getResources().getString(
                collapsed ? R.string.expand_top_bar : R.string.collapse_top_bar));
        editAction.setText(getResources().getString(
                toolsExpanded ? R.string.finish_editing : R.string.edit_tools));
        headerRow.setPadding(dp(12), collapsed ? dp(3) : dp(7), dp(8),
                collapsed ? dp(3) : dp(5));
        title.setTextSize(collapsed ? 16 : 18);
    }

    private void addTool(LinearLayout parent, AnnotationStore.Tool tool, int label) {
        TextView action = toolAction(label, () -> listener.onToolSelected(tool));
        toolActions.put(tool, action);
        parent.addView(action, wrapWithMargins(3));
    }

    private TextView toolAction(int label, Runnable runnable) {
        TextView action = action(getResources().getString(label), runnable);
        action.setTextSize(13);
        styleAction(action, WHITE, INK, Color.rgb(215, 222, 227));
        return action;
    }

    private TextView action(String label, Runnable runnable) {
        TextView view = text(label, 13, Typeface.BOLD, INK);
        view.setGravity(Gravity.CENTER);
        view.setMinHeight(dp(48));
        view.setMinWidth(dp(48));
        view.setPadding(dp(11), 0, dp(11), 0);
        view.setClickable(true);
        view.setFocusable(true);
        view.setOnClickListener(ignored -> runnable.run());
        return view;
    }

    private TextView text(String value, float size, int style, int color) {
        TextView view = new TextView(getContext());
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setTypeface(Typeface.create("sans", style));
        return view;
    }

    private void addWeightedAction(LinearLayout parent, TextView view) {
        LinearLayout.LayoutParams params =
                new LinearLayout.LayoutParams(0, dp(48), 1f);
        params.setMargins(dp(3), 0, dp(3), 0);
        parent.addView(view, params);
    }

    private void styleAction(TextView view, int background, int foreground, int stroke) {
        view.setTextColor(foreground);
        view.setBackground(ripple(background, stroke, 14));
    }

    private android.graphics.drawable.Drawable ripple(int fill, int stroke, int radius) {
        GradientDrawable content = roundRect(fill, stroke, radius);
        return new RippleDrawable(
                ColorStateList.valueOf(withAlpha(Color.WHITE, 56)), content, null);
    }

    private GradientDrawable roundRect(int fill, int stroke, int radius) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(fill);
        drawable.setCornerRadius(dp(radius));
        drawable.setStroke(dp(1), stroke);
        return drawable;
    }

    private static int withAlpha(int color, int alpha) {
        return Color.argb(alpha, Color.red(color), Color.green(color), Color.blue(color));
    }

    private void setEnabledState(TextView view, boolean enabled) {
        view.setEnabled(enabled);
        view.setClickable(enabled);
        view.setAlpha(enabled ? 1f : 0.38f);
    }

    private LinearLayout.LayoutParams weighted() {
        return new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f);
    }

    private LinearLayout.LayoutParams matchWrap() {
        return new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    private LinearLayout.LayoutParams sized(int width, int height) {
        return new LinearLayout.LayoutParams(width, height);
    }

    private LinearLayout.LayoutParams sizedWithMargins(int width, int height, int margin) {
        LinearLayout.LayoutParams params = sized(width, height);
        params.setMargins(dp(margin), 0, dp(margin), 0);
        return params;
    }

    private LinearLayout.LayoutParams wrapWithMargins(int margin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, dp(48));
        params.setMargins(dp(margin), 0, dp(margin), 0);
        return params;
    }

    private static int dp(int value) {
        return Math.round(value
                * android.content.res.Resources.getSystem().getDisplayMetrics().density);
    }
}
