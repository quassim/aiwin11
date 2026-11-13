using System.IO;
using System.IO.Compression;
using System.Text;
using System.Xml.Linq;

/// Extracts bounded UTF-8 plain text from Office Open XML (.docx, .xlsx, .pptx) and ODF (.odt)
/// documents inside an already broker-authorized path. Standard-user read-only.
internal static class DocumentReader
{
    const long MaxFileBytes = 2 * 1024 * 1024;
    const int MaxChars = 16_000;

    public static string? ExtractText(string path, long deadlineMs)
    {
        if (DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadlineMs) return null;
        if (string.IsNullOrWhiteSpace(path) || !File.Exists(path)) return null;
        var info = new FileInfo(path);
        if (info.Length <= 0 || info.Length > MaxFileBytes) return null;
        if ((info.Attributes & FileAttributes.ReparsePoint) != 0) return null;

        var ext = Path.GetExtension(path).ToLowerInvariant();
        try
        {
            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            using var archive = new ZipArchive(stream, ZipArchiveMode.Read);
            return ext switch
            {
                ".docx" => ReadDocx(archive),
                ".xlsx" => ReadXlsx(archive),
                ".pptx" => ReadPptx(archive),
                ".odt" => ReadOdt(archive),
                _ => null,
            };
        }
        catch (Exception error) when (error is InvalidDataException or IOException or UnauthorizedAccessException or System.Xml.XmlException)
        {
            return null;
        }
    }

    static string? ReadDocx(ZipArchive archive)
    {
        var entry = archive.GetEntry("word/document.xml");
        if (entry is null) return null;
        using var stream = entry.Open();
        var doc = XDocument.Load(stream);
        XNamespace w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        var sb = new StringBuilder();
        foreach (var paragraph in doc.Descendants(w + "p"))
        {
            var line = string.Concat(paragraph.Descendants(w + "t").Select(t => t.Value)).Trim();
            if (line.Length == 0) continue;
            if (!AppendLine(sb, line)) break;
        }
        return sb.ToString().Trim();
    }

    static string? ReadXlsx(ZipArchive archive)
    {
        var shared = new List<string>();
        var sharedEntry = archive.GetEntry("xl/sharedStrings.xml");
        if (sharedEntry is not null)
        {
            using var ss = sharedEntry.Open();
            var doc = XDocument.Load(ss);
            XNamespace s = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
            foreach (var si in doc.Descendants(s + "si"))
            {
                shared.Add(string.Concat(si.Descendants(s + "t").Select(t => t.Value)));
            }
        }
        XNamespace main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        var sb = new StringBuilder();
        var sheets = archive.Entries
            .Where(e => e.FullName.StartsWith("xl/worksheets/sheet", StringComparison.OrdinalIgnoreCase) && e.FullName.EndsWith(".xml", StringComparison.OrdinalIgnoreCase))
            .OrderBy(e => e.FullName, StringComparer.OrdinalIgnoreCase)
            .Take(5);
        foreach (var sheet in sheets)
        {
            if (!AppendLine(sb, $"[{Path.GetFileNameWithoutExtension(sheet.Name)}]")) break;
            using var stream = sheet.Open();
            var doc = XDocument.Load(stream);
            foreach (var row in doc.Descendants(main + "row"))
            {
                var cells = new List<string>();
                foreach (var cell in row.Elements(main + "c"))
                {
                    var type = (string?)cell.Attribute("t");
                    var val = cell.Element(main + "v")?.Value ?? string.Concat(cell.Descendants(main + "t").Select(t => t.Value));
                    if (type == "s" && int.TryParse(val, out var idx) && idx >= 0 && idx < shared.Count)
                    {
                        val = shared[idx];
                    }
                    if (!string.IsNullOrWhiteSpace(val)) cells.Add(val.Trim());
                }
                if (cells.Count > 0 && !AppendLine(sb, string.Join(" | ", cells))) return sb.ToString().Trim();
            }
        }
        return sb.ToString().Trim();
    }

    static string? ReadPptx(ZipArchive archive)
    {
        XNamespace a = "http://schemas.openxmlformats.org/drawingml/2006/main";
        var sb = new StringBuilder();
        var slides = archive.Entries
            .Where(e => e.FullName.StartsWith("ppt/slides/slide", StringComparison.OrdinalIgnoreCase) && e.FullName.EndsWith(".xml", StringComparison.OrdinalIgnoreCase))
            .OrderBy(e => e.FullName, StringComparer.OrdinalIgnoreCase)
            .Take(30);
        foreach (var slide in slides)
        {
            using var stream = slide.Open();
            var doc = XDocument.Load(stream);
            var lines = doc.Descendants(a + "p")
                .Select(p => string.Concat(p.Descendants(a + "t").Select(t => t.Value)).Trim())
                .Where(t => t.Length > 0)
                .ToList();
            if (lines.Count == 0) continue;
            if (!AppendLine(sb, $"[{Path.GetFileNameWithoutExtension(slide.Name)}]")) break;
            foreach (var line in lines)
            {
                if (!AppendLine(sb, line)) return sb.ToString().Trim();
            }
        }
        return sb.ToString().Trim();
    }

    static string? ReadOdt(ZipArchive archive)
    {
        var entry = archive.GetEntry("content.xml");
        if (entry is null) return null;
        using var stream = entry.Open();
        var doc = XDocument.Load(stream);
        XNamespace text = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
        var sb = new StringBuilder();
        foreach (var node in doc.Descendants().Where(e => e.Name == text + "p" || e.Name == text + "h"))
        {
            var line = node.Value.Trim();
            if (line.Length == 0) continue;
            if (!AppendLine(sb, line)) break;
        }
        return sb.ToString().Trim();
    }

    static bool AppendLine(StringBuilder sb, string line)
    {
        if (sb.Length >= MaxChars) return false;
        var remaining = MaxChars - sb.Length;
        if (line.Length >= remaining)
        {
            sb.Append(line.AsSpan(0, remaining));
            return false;
        }
        sb.AppendLine(line);
        return true;
    }
}
