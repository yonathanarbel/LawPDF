package com.lawpdf.mobile;

import android.annotation.SuppressLint;
import android.content.Context;
import android.graphics.Color;
import android.view.View;
import android.view.ViewGroup;
import android.widget.BaseAdapter;
import android.widget.ListView;
import android.widget.TextView;

import java.util.ArrayList;
import java.util.List;

/** A virtualized DOCX text view: only visible paragraphs own Android Views. */
@SuppressLint("ViewConstructor")
final class DocxParagraphList extends ListView {
    private static final int MAX_ROW_CHARS = 8_000;
    private final int paragraphCount;

    DocxParagraphList(Context context, String text) {
        super(context);
        List<String> paragraphs = paragraphs(text);
        paragraphCount = paragraphs.size();
        setBackgroundColor(Color.rgb(244, 241, 234));
        setDividerHeight(0);
        setClipToPadding(false);
        setPadding(dp(context, 12), dp(context, 10), dp(context, 12), dp(context, 36));
        setAdapter(new ParagraphAdapter(context, paragraphs));
    }

    int paragraphCount() {
        return paragraphCount;
    }

    private static List<String> paragraphs(String text) {
        ArrayList<String> result = new ArrayList<>();
        String normalized = text.replace("\r\n", "\n").replace('\r', '\n');
        for (String paragraph : normalized.split("\n", -1)) {
            if (paragraph.isEmpty()) continue;
            for (int start = 0; start < paragraph.length(); start += MAX_ROW_CHARS) {
                int end = Math.min(start + MAX_ROW_CHARS, paragraph.length());
                if (end < paragraph.length()
                        && Character.isHighSurrogate(paragraph.charAt(end - 1))
                        && Character.isLowSurrogate(paragraph.charAt(end))) {
                    end--;
                }
                result.add(paragraph.substring(start, end));
                start = end - MAX_ROW_CHARS;
            }
        }
        if (result.isEmpty()) result.add("");
        return result;
    }

    private static final class ParagraphAdapter extends BaseAdapter {
        private final Context context;
        private final List<String> paragraphs;

        ParagraphAdapter(Context context, List<String> paragraphs) {
            this.context = context;
            this.paragraphs = paragraphs;
        }

        @Override public int getCount() { return paragraphs.size(); }
        @Override public String getItem(int position) { return paragraphs.get(position); }
        @Override public long getItemId(int position) { return position; }
        @Override public boolean hasStableIds() { return true; }

        @Override
        public View getView(int position, View recycled, ViewGroup parent) {
            TextView view = recycled instanceof TextView ? (TextView) recycled : new TextView(context);
            view.setText(getItem(position));
            view.setTextColor(Color.rgb(30, 30, 30));
            view.setTextSize(17);
            view.setLineSpacing(dp(context, 4), 1.0f);
            view.setPadding(dp(context, 10), dp(context, 7), dp(context, 10), dp(context, 7));
            view.setTextIsSelectable(true);
            return view;
        }
    }

    private static int dp(Context context, int value) {
        return Math.round(value * context.getResources().getDisplayMetrics().density);
    }
}
