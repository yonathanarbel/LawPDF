package com.lawpdf.mobile;

import android.util.AtomicFile;
import java.io.DataInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

/** Complete, versioned annotation snapshots committed before an edit is accepted. */
final class AnnotationJournal {
    private static final int MAGIC = 0x4c504446;
    private static final int VERSION = 1;
    private static final int MAX_STROKES = 10000;
    private static final int MAX_POINTS = 100000;
    private final AtomicFile file;

    AnnotationJournal(File directory, String revision) throws IOException {
        if (!revision.matches("[a-f0-9]{64}")) throw new IOException("Invalid PDF revision.");
        if (!directory.isDirectory() && !directory.mkdirs()) throw new IOException("Could not create the annotation folder.");
        file = new AtomicFile(new File(directory, revision + ".marks"));
    }

    List<AnnotationStore.Stroke> read(int pageCount) throws IOException {
        ArrayList<AnnotationStore.Stroke> strokes = new ArrayList<>();
        try (DataInputStream input = new DataInputStream(file.openRead())) {
            if (input.readInt() != MAGIC || input.readInt() != VERSION) throw new IOException("These annotations need a compatible LawPDF version. They were kept unchanged.");
            int count = input.readInt();
            if (count < 0 || count > MAX_STROKES) throw new IOException("Invalid annotation count.");
            int totalPoints = 0;
            for (int index = 0; index < count; index++) {
                int page = input.readInt();
                int tool = input.readInt();
                int color = input.readInt();
                float width = input.readFloat();
                int pointCount = input.readInt();
                if (page < 0 || page >= pageCount || tool < 0 || tool >= AnnotationStore.Tool.values().length
                        || !finite(width) || width <= 0f || width > 1f || pointCount < 1 || pointCount > MAX_POINTS - totalPoints) {
                    throw new IOException("An annotation record is damaged. It was kept unchanged.");
                }
                totalPoints += pointCount;
                AnnotationStore.Stroke stroke = new AnnotationStore.Stroke(page, AnnotationStore.Tool.values()[tool], color, width);
                for (int point = 0; point < pointCount; point++) {
                    float x = input.readFloat(), y = input.readFloat();
                    if (!finite(x) || !finite(y) || x < 0f || x > 1f || y < 0f || y > 1f) throw new IOException("Invalid annotation coordinates.");
                    stroke.points.add(new AnnotationStore.Point(x, y));
                }
                strokes.add(stroke);
            }
            if (input.read() != -1) throw new IOException("Unexpected annotation data. The record was kept unchanged.");
        } catch (FileNotFoundException absent) {
            if (file.getBaseFile().exists()) throw absent;
        }
        return strokes;
    }

    private static boolean finite(float value) { return !Float.isNaN(value) && !Float.isInfinite(value); }

    void write(List<AnnotationStore.Stroke> strokes) throws IOException {
        if (strokes.size() > MAX_STROKES) throw new IOException("The annotation limit was reached. Export a copy before adding more marks.");
        long points = 0;
        for (AnnotationStore.Stroke stroke : strokes) points += stroke.points.size();
        if (points > MAX_POINTS) throw new IOException("The drawing limit was reached. Export a copy before adding more marks.");
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        output.writeInt(MAGIC); output.writeInt(VERSION); output.writeInt(strokes.size());
        for (AnnotationStore.Stroke stroke : strokes) {
            output.writeInt(stroke.page); output.writeInt(stroke.tool.ordinal()); output.writeInt(stroke.color);
            output.writeFloat(stroke.width); output.writeInt(stroke.points.size());
            for (AnnotationStore.Point point : stroke.points) { output.writeFloat(point.x); output.writeFloat(point.y); }
        }
        output.flush();
        byte[] expected = bytes.toByteArray();
        FileOutputStream stream = file.startWrite();
        try {
            stream.write(expected);
            stream.getFD().sync();
            file.finishWrite(stream);
        } catch (IOException | RuntimeException error) {
            file.failWrite(stream);
            throw error;
        }
        // AtomicFile's finish method reports some rename failures only through
        // Android logging. Verify the committed bytes before acknowledging.
        try (java.io.InputStream input = file.openRead()) {
            byte[] actual = new byte[expected.length];
            int offset = 0;
            while (offset < actual.length) {
                int count = input.read(actual, offset, actual.length - offset);
                if (count < 0) break;
                offset += count;
            }
            if (offset != actual.length || input.read() != -1 || !java.util.Arrays.equals(expected, actual)) {
                throw new IOException("The annotation save could not be confirmed. Free some device storage and try again.");
            }
        }
    }
}
