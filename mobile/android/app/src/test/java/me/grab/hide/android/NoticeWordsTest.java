package me.grab.hide.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.HashMap;
import java.util.Map;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.junit.Test;

/**
 * The Android table equals the iOS app's ({@code NoticeWords.swift}) key for key in every language, read from the iOS
 * source itself, so the two apps word a notice the same way. The web side holds the iOS table equal to the page's catalog
 * ({@code web/src/mobile/notificationWords.test.ts}), so all three agree. Gradle names the file
 * ({@code hide.iosNoticeWords}); a build without it fails here rather than passing on nothing.
 */
public class NoticeWordsTest {

    private static Map<String, Map<String, String>> swiftTables() throws Exception {
        String path = System.getProperty("hide.iosNoticeWords");
        assertNotNull("app/build.gradle names the iOS words file", path);
        String source = new String(Files.readAllBytes(Paths.get(path)), StandardCharsets.UTF_8);
        Map<String, Map<String, String>> tables = new TreeMap<>();
        Matcher language = Pattern.compile("\"([a-z]{2})\": \\[\\n([\\s\\S]*?)\\n {8}\\],").matcher(source);
        while (language.find()) {
            Map<String, String> entries = new TreeMap<>();
            Matcher entry = Pattern.compile("\"([a-z_]+)\": \"((?:[^\"\\\\]|\\\\.)*)\"").matcher(language.group(2));
            while (entry.find()) entries.put(entry.group(1), entry.group(2));
            tables.put(language.group(1), entries);
        }
        return tables;
    }

    @Test
    public void theTablesAreTheIosAppsKeyForKey() throws Exception {
        Map<String, Map<String, String>> ios = swiftTables();
        Map<String, Map<String, String>> android = new TreeMap<>();
        for (Map.Entry<String, Map<String, String>> table : NoticeWords.TABLES.entrySet()) android.put(table.getKey(), new TreeMap<>(table.getValue()));
        assertEquals(java.util.List.of("en", "ja", "ko", "zh"), new java.util.ArrayList<>(ios.keySet()));
        assertEquals(15, ios.get("en").size());
        assertEquals(ios, android);
    }

    @Test
    public void leadsWithTheStatusWordElseTheGroup() {
        NoticeWords english = NoticeWords.forLanguages(java.util.List.of("en"));
        assertEquals("Question", english.lead("question", "needs_you"));
        assertEquals("Needs You", english.lead("not_a_status", "needs_you"));
        assertEquals("Needs You", english.lead(null, "needs_you"));
        assertEquals(null, english.lead(null, "not_a_group"));
        Map<String, String> none = new HashMap<>();
        assertEquals(null, new NoticeWords(none).lead("question", "needs_you"));
    }
}
