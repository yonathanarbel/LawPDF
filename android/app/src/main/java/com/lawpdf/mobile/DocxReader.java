package com.lawpdf.mobile;

import org.xml.sax.Attributes;
import org.xml.sax.SAXException;
import org.xml.sax.helpers.DefaultHandler;

import java.io.FilterInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.Locale;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

import javax.xml.parsers.ParserConfigurationException;
import javax.xml.parsers.SAXParserFactory;

/** Streaming, dependency-free text extraction for Office Open XML documents. */
final class DocxReader {
    private static final long MAX_DOCUMENT_XML_BYTES = 32L * 1024L * 1024L;
    private static final int MAX_TEXT_CHARS = 4_000_000;

    private DocxReader() {}

    static String read(InputStream source) throws IOException {
        try (ZipInputStream zip = new ZipInputStream(source)) {
            ZipEntry entry;
            while ((entry = zip.getNextEntry()) != null) {
                String normalized = entry.getName().replace('\\', '/').toLowerCase(Locale.ROOT);
                if (normalized.equals("word/document.xml") || normalized.equals("/word/document.xml")) {
                    return parseDocumentXml(new LimitedInputStream(zip, MAX_DOCUMENT_XML_BYTES));
                }
            }
        }
        throw new IOException("This DOCX package has no word/document.xml file.");
    }

    private static String parseDocumentXml(InputStream xml) throws IOException {
        SAXParserFactory factory = SAXParserFactory.newInstance();
        factory.setNamespaceAware(true);
        trySetFeature(factory, "http://apache.org/xml/features/disallow-doctype-decl", true);
        trySetFeature(factory, "http://xml.org/sax/features/external-general-entities", false);
        trySetFeature(factory, "http://xml.org/sax/features/external-parameter-entities", false);

        WordHandler handler = new WordHandler();
        try {
            factory.newSAXParser().parse(xml, handler);
        } catch (ParserConfigurationException | SAXException error) {
            throw new IOException("Could not parse DOCX document XML.", error);
        }
        String result = handler.result();
        if (result.trim().isEmpty()) {
            throw new IOException("The DOCX document contains no readable text.");
        }
        return result;
    }

    private static void trySetFeature(SAXParserFactory factory, String name, boolean value) {
        try {
            factory.setFeature(name, value);
        } catch (ParserConfigurationException | SAXException ignored) {
            // Android parser implementations differ; namespace-aware parsing and
            // the absence of a custom entity resolver remain the safe fallback.
        }
    }

    private static final class WordHandler extends DefaultHandler {
        private final StringBuilder text = new StringBuilder();
        private boolean inText;

        @Override
        public void startElement(String uri, String localName, String qName, Attributes attributes)
                throws SAXException {
            String name = elementName(localName, qName);
            if (name.equals("t")) {
                inText = true;
            } else if (name.equals("tab")) {
                append("\t");
            } else if (name.equals("br") || name.equals("cr")) {
                newline();
            }
        }

        @Override
        public void characters(char[] chars, int start, int length) throws SAXException {
            if (inText) {
                append(new String(chars, start, length));
            }
        }

        @Override
        public void endElement(String uri, String localName, String qName) {
            String name = elementName(localName, qName);
            if (name.equals("t")) {
                inText = false;
            } else if (name.equals("p")) {
                newline();
            } else if (name.equals("tc") && text.length() > 0) {
                char last = text.charAt(text.length() - 1);
                if (last != '\n' && last != '\t') {
                    text.append('\t');
                }
            }
        }

        String result() {
            String raw = text.toString().replace("\r\n", "\n").replace('\r', '\n');
            StringBuilder collapsed = new StringBuilder(raw.length());
            boolean priorBlank = false;
            for (String line : raw.split("\n", -1)) {
                boolean blank = line.trim().isEmpty();
                if (!blank || !priorBlank) {
                    if (collapsed.length() > 0) collapsed.append('\n');
                    collapsed.append(stripTrailingWhitespace(line));
                }
                priorBlank = blank;
            }
            return collapsed.toString().trim();
        }

        private void append(String value) throws SAXException {
            if (text.length() + value.length() > MAX_TEXT_CHARS) {
                throw new SAXException("DOCX text exceeds the supported size limit.");
            }
            text.append(value);
        }

        private void newline() {
            if (text.length() > 0 && text.charAt(text.length() - 1) != '\n') {
                text.append('\n');
            }
        }

        private static String elementName(String localName, String qName) {
            String value = localName == null || localName.isEmpty() ? qName : localName;
            int colon = value.indexOf(':');
            return colon >= 0 ? value.substring(colon + 1) : value;
        }

        private static String stripTrailingWhitespace(String line) {
            int end = line.length();
            while (end > 0 && Character.isWhitespace(line.charAt(end - 1))) end--;
            return line.substring(0, end);
        }
    }

    private static final class LimitedInputStream extends FilterInputStream {
        private final long limit;
        private long read;

        LimitedInputStream(InputStream input, long limit) {
            super(input);
            this.limit = limit;
        }

        @Override
        public int read() throws IOException {
            int value = super.read();
            if (value >= 0) account(1);
            return value;
        }

        @Override
        public int read(byte[] buffer, int offset, int length) throws IOException {
            int count = super.read(buffer, offset, length);
            if (count > 0) account(count);
            return count;
        }

        private void account(int count) throws IOException {
            read += count;
            if (read > limit) throw new IOException("DOCX document XML is too large.");
        }
    }
}
