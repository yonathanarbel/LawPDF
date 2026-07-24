package com.lawpdf.mobile;

import org.junit.Test;

import java.util.List;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public final class AnnotationStoreTest {
    @Test
    public void addUndoRedoPreservesStroke() {
        AnnotationStore store = new AnnotationStore();
        AnnotationStore.Stroke stroke = store.startStroke(
                2, AnnotationStore.Tool.PEN, 0xff000000, 0.004f, 0.1f, 0.2f);
        stroke.add(0.7f, 0.8f);
        store.commit(stroke);

        assertEquals(1, store.size());
        assertTrue(store.canUndo());
        assertTrue(store.undo());
        assertEquals(0, store.size());
        assertTrue(store.canRedo());
        assertTrue(store.redo());
        assertEquals(1, store.size());
        assertEquals(2, store.forPage(2).get(0).page);
    }

    @Test
    public void eraserRemovesOnlyNearbyStrokeAndIsUndoable() {
        AnnotationStore store = new AnnotationStore();
        AnnotationStore.Stroke first = store.startStroke(
                0, AnnotationStore.Tool.PEN, 1, 0.004f, 0.1f, 0.1f);
        first.add(0.9f, 0.1f);
        store.commit(first);
        AnnotationStore.Stroke second = store.startStroke(
                1, AnnotationStore.Tool.PEN, 2, 0.004f, 0.1f, 0.8f);
        second.add(0.9f, 0.8f);
        store.commit(second);

        assertFalse(store.eraseAt(0, 0.5f, 0.5f, 0.02f));
        assertTrue(store.eraseAt(0, 0.5f, 0.11f, 0.02f));
        assertEquals(0, store.forPage(0).size());
        assertEquals(1, store.forPage(1).size());
        assertTrue(store.undo());
        assertEquals(1, store.forPage(0).size());
    }

    @Test
    public void snapshotIsIndependentAndCoordinatesAreClamped() {
        AnnotationStore store = new AnnotationStore();
        AnnotationStore.Stroke stroke = store.startStroke(
                0, AnnotationStore.Tool.HIGHLIGHT, 3, 0.02f, -1f, 2f);
        store.commit(stroke);
        List<AnnotationStore.Stroke> snapshot = store.snapshot();

        assertEquals(0f, snapshot.get(0).points.get(0).x, 0f);
        assertEquals(1f, snapshot.get(0).points.get(0).y, 0f);
        assertTrue(store.undo());
        assertEquals(1, snapshot.size());
        assertEquals(0, store.size());
    }
}
