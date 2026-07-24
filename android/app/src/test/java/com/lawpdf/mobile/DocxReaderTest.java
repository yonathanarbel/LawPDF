package com.lawpdf.mobile;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

public final class DocxReaderTest {
    @Test
    public void extractsParagraphsTablesTabsBreaksAndEntities() throws Exception {
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
                + "<w:document xmlns:w=\"urn:test\"><w:body>"
                + "<w:p><w:r><w:t>Hello &amp; goodbye</w:t></w:r></w:p>"
                + "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Left</w:t></w:r></w:p></w:tc>"
                + "<w:tc><w:p><w:r><w:t>Right</w:t><w:tab/><w:t>cell</w:t>"
                + "<w:br/><w:t>line</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"
                + "</w:body></w:document>";

        assertEquals("Hello & goodbye\nLeft\nRight\tcell\nline", DocxReader.read(docx(xml)));
    }

    @Test
    public void rejectsPackagesWithoutDocumentXml() throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
            zip.putNextEntry(new ZipEntry("[Content_Types].xml"));
            zip.write("<Types/>".getBytes(StandardCharsets.UTF_8));
        }
        assertThrows(IOException.class,
                () -> DocxReader.read(new ByteArrayInputStream(bytes.toByteArray())));
    }

    private static ByteArrayInputStream docx(String xml) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
            zip.putNextEntry(new ZipEntry("word/document.xml"));
            zip.write(xml.getBytes(StandardCharsets.UTF_8));
        }
        return new ByteArrayInputStream(bytes.toByteArray());
    }
}
