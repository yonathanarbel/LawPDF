package com.lawpdf.mobile;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Deque;
import java.util.List;

/** Page-normalized freehand annotations with reversible edits. */
final class AnnotationStore {
    enum Tool { PAN, PEN, HIGHLIGHT, SIGNATURE, ERASER }

    static final class Point {
        final float x;
        final float y;

        Point(float x, float y) {
            this.x = clamp01(x);
            this.y = clamp01(y);
        }
    }

    static final class Stroke {
        final int page;
        final Tool tool;
        final int color;
        final float width;
        final ArrayList<Point> points = new ArrayList<>();

        Stroke(int page, Tool tool, int color, float width) {
            this.page = page;
            this.tool = tool;
            this.color = color;
            this.width = width;
        }

        Stroke(Stroke source) {
            page = source.page;
            tool = source.tool;
            color = source.color;
            width = source.width;
            for (Point point : source.points) points.add(new Point(point.x, point.y));
        }

        void add(float x, float y) {
            Point point = new Point(x, y);
            if (!points.isEmpty()) {
                Point previous = points.get(points.size() - 1);
                float dx = point.x - previous.x;
                float dy = point.y - previous.y;
                if (dx * dx + dy * dy < 0.000001f) return;
            }
            points.add(point);
        }
    }

    private interface Change {
        void undo(List<Stroke> strokes);
        void redo(List<Stroke> strokes);
    }

    private static final class AddChange implements Change {
        private final Stroke stroke;
        private final int index;

        AddChange(Stroke stroke, int index) {
            this.stroke = stroke;
            this.index = index;
        }

        @Override public void undo(List<Stroke> strokes) { strokes.remove(stroke); }
        @Override public void redo(List<Stroke> strokes) {
            strokes.add(Math.min(index, strokes.size()), stroke);
        }
    }

    private static final class RemoveChange implements Change {
        private final Stroke stroke;
        private final int index;

        RemoveChange(Stroke stroke, int index) {
            this.stroke = stroke;
            this.index = index;
        }

        @Override public void undo(List<Stroke> strokes) {
            strokes.add(Math.min(index, strokes.size()), stroke);
        }
        @Override public void redo(List<Stroke> strokes) { strokes.remove(stroke); }
    }

    private final ArrayList<Stroke> strokes = new ArrayList<>();
    private final Deque<Change> undo = new ArrayDeque<>();
    private final Deque<Change> redo = new ArrayDeque<>();

    Stroke startStroke(int page, Tool tool, int color, float width, float x, float y) {
        Stroke stroke = new Stroke(page, tool, color, width);
        stroke.add(x, y);
        return stroke;
    }

    void commit(Stroke stroke) {
        if (stroke == null || stroke.points.isEmpty()) return;
        int index = strokes.size();
        strokes.add(stroke);
        undo.addLast(new AddChange(stroke, index));
        redo.clear();
    }

    boolean eraseAt(int page, float x, float y, float radius) {
        for (int index = strokes.size() - 1; index >= 0; index--) {
            Stroke stroke = strokes.get(index);
            if (stroke.page != page) continue;
            float tolerance = radius + stroke.width * 0.5f;
            if (distanceToStroke(stroke, x, y) <= tolerance) {
                strokes.remove(index);
                undo.addLast(new RemoveChange(stroke, index));
                redo.clear();
                return true;
            }
        }
        return false;
    }

    boolean undo() {
        Change change = undo.pollLast();
        if (change == null) return false;
        change.undo(strokes);
        redo.addLast(change);
        return true;
    }

    boolean redo() {
        Change change = redo.pollLast();
        if (change == null) return false;
        change.redo(strokes);
        undo.addLast(change);
        return true;
    }

    boolean canUndo() { return !undo.isEmpty(); }
    boolean canRedo() { return !redo.isEmpty(); }
    int size() { return strokes.size(); }

    List<Stroke> forPage(int page) {
        ArrayList<Stroke> result = new ArrayList<>();
        for (Stroke stroke : strokes) if (stroke.page == page) result.add(stroke);
        return result;
    }

    List<Stroke> snapshot() {
        ArrayList<Stroke> result = new ArrayList<>(strokes.size());
        for (Stroke stroke : strokes) result.add(new Stroke(stroke));
        return Collections.unmodifiableList(result);
    }

    void clear() {
        strokes.clear();
        undo.clear();
        redo.clear();
    }

    private static float distanceToStroke(Stroke stroke, float x, float y) {
        if (stroke.points.isEmpty()) return Float.MAX_VALUE;
        if (stroke.points.size() == 1) {
            Point point = stroke.points.get(0);
            return distance(x, y, point.x, point.y);
        }
        float best = Float.MAX_VALUE;
        for (int index = 1; index < stroke.points.size(); index++) {
            Point start = stroke.points.get(index - 1);
            Point end = stroke.points.get(index);
            best = Math.min(best, distanceToSegment(x, y, start.x, start.y, end.x, end.y));
        }
        return best;
    }

    private static float distanceToSegment(
            float x, float y, float startX, float startY, float endX, float endY) {
        float dx = endX - startX;
        float dy = endY - startY;
        float lengthSquared = dx * dx + dy * dy;
        if (lengthSquared == 0f) return distance(x, y, startX, startY);
        float position = ((x - startX) * dx + (y - startY) * dy) / lengthSquared;
        position = Math.max(0f, Math.min(1f, position));
        return distance(x, y, startX + position * dx, startY + position * dy);
    }

    private static float distance(float x1, float y1, float x2, float y2) {
        float dx = x1 - x2;
        float dy = y1 - y2;
        return (float) Math.sqrt(dx * dx + dy * dy);
    }

    private static float clamp01(float value) {
        return Math.max(0f, Math.min(1f, value));
    }
}
