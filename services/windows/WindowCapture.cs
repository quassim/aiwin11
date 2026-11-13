using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Google.Protobuf;
using Protocol = Aiwin11.V1;

internal static class WindowCapture
{
    private const int PreviewSide = 1024;
    private const int MaxJpegBytes = 256 * 1024;
    private const uint RenderFullContent = 2;
    private const uint Blackness = 0x00000042;
    private const uint SourceCopy = 0x00CC0020;

    internal static Protocol.WindowCapture? ReadDisplay(Protocol.DisplayInfo display, long deadline)
    {
        if (display.Width == 0 || display.Height == 0 || (ulong)display.Width * display.Height > 36_000_000)
            return null;
        nint previousDpi = SetThreadDpiAwarenessContext(-4);
        if (previousDpi == 0)
            return null;
        try
        {
            bool StillPresent() => Displays.Read()?.Displays.Contains(display) == true;
            if (!StillPresent())
                return null;
            nint screen = GetDC(0);
            if (screen == 0)
                return null;
            try
            {
                int width = (int)display.Width;
                int height = (int)display.Height;
                return Render(screen, width, height, deadline, StillPresent,
                    memory => BitBlt(memory, 0, 0, width, height, screen, display.X, display.Y, SourceCopy));
            }
            finally { _ = ReleaseDC(0, screen); }
        }
        finally { SetThreadDpiAwarenessContext(previousDpi); }
    }

    /// <summary>Foreground window, for the desktop Inspector page.</summary>
    internal static Protocol.WindowCapture? Read(long deadline)
    {
        nint window = GetForegroundWindow();
        return window == 0 ? null : Capture(window, deadline, () => GetForegroundWindow() == window);
    }

    /// <summary>
    /// Exactly this window, for the agent after the user approved it. It does not need to be
    /// in the foreground, but must still belong to the same process before and after rendering.
    /// </summary>
    internal static Protocol.WindowCapture? ReadWindow(ulong handle, uint processId, long deadline)
    {
        nint window = (nint)handle;
        bool Owned() => IsWindow(window) && GetWindowThreadProcessId(window, out uint pid) != 0 && pid == processId;
        return window == 0 || !Owned() ? null : Capture(window, deadline, Owned);
    }

    private static Protocol.WindowCapture? Capture(nint window, long deadline, Func<bool> stillValid)
    {
        nint previousDpi = SetThreadDpiAwarenessContext(-4);
        try
        {
            // Windows that exclude themselves from capture (display affinity) are never read.
            if (!GetWindowDisplayAffinity(window, out uint affinity) || affinity != 0
                || !GetWindowRect(window, out var rect))
                return null;

            int width = rect.Right - rect.Left;
            int height = rect.Bottom - rect.Top;
            if (width <= 0 || height <= 0 || (long)width * height > 16_000_000)
                return null;

            nint screen = GetDC(0);
            if (screen == 0)
                return null;

            bool CanCapture() => stillValid() && GetWindowDisplayAffinity(window, out uint current) && current == 0;
            try
            {
                return Render(screen, width, height, deadline, CanCapture,
                    memory => PrintWindow(window, memory, RenderFullContent));
            }
            finally { _ = ReleaseDC(0, screen); }
        }
        finally
        {
            if (previousDpi != 0) SetThreadDpiAwarenessContext(previousDpi);
        }
    }

    private static Protocol.WindowCapture? Render(
        nint screen, int width, int height, long deadline, Func<bool> stillValid, Func<nint, bool> copy)
    {
        nint memory = CreateCompatibleDC(screen);
        if (memory == 0) return null;
        nint bitmap = 0;
        nint original = 0;
        try
        {
            bitmap = CreateCompatibleBitmap(screen, width, height);
            if (bitmap == 0) return null;
            original = SelectObject(memory, bitmap);
            if (original == 0 || original == -1 || !PatBlt(memory, 0, 0, width, height, Blackness))
                return null;
            if (!stillValid() || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline
                || !copy(memory) || !stillValid())
                return null;

            SelectObject(memory, original);
            original = 0;
            var source = Imaging.CreateBitmapSourceFromHBitmap(bitmap, 0, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
            double scale = Math.Min(1.0, (double)PreviewSide / Math.Max(width, height));
            BitmapSource preview = scale < 1 ? new TransformedBitmap(source, new ScaleTransform(scale, scale)) : source;
            var pixels = new FormatConvertedBitmap(preview, PixelFormats.Bgr24, null, 0);
            var buffer = new byte[pixels.PixelWidth * pixels.PixelHeight * 3];
            pixels.CopyPixels(buffer, pixels.PixelWidth * 3, 0);
            if (!buffer.Any(component => component != 0)) return null;

            using var stream = new MemoryStream();
            var encoder = new JpegBitmapEncoder { QualityLevel = 72 };
            encoder.Frames.Add(BitmapFrame.Create(pixels));
            encoder.Save(stream);
            if (stream.Length > MaxJpegBytes || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline
                || !stillValid())
                return null;

            return new Protocol.WindowCapture {
                Jpeg = ByteString.CopyFrom(stream.ToArray()),
                Width = (uint)pixels.PixelWidth,
                Height = (uint)pixels.PixelHeight,
            };
        }
        finally
        {
            if (original != 0 && original != -1) SelectObject(memory, original);
            if (bitmap != 0) DeleteObject(bitmap);
            DeleteDC(memory);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(nint window, out Rect rect);
    [DllImport("user32.dll")] private static extern bool GetWindowDisplayAffinity(nint window, out uint affinity);
    [DllImport("user32.dll")] private static extern nint SetThreadDpiAwarenessContext(nint context);
    [DllImport("user32.dll")] private static extern nint GetDC(nint window);
    [DllImport("user32.dll")] private static extern int ReleaseDC(nint window, nint dc);
    [DllImport("user32.dll")] private static extern bool PrintWindow(nint window, nint dc, uint flags);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleDC(nint dc);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleBitmap(nint dc, int width, int height);
    [DllImport("gdi32.dll")] private static extern nint SelectObject(nint dc, nint obj);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(nint obj);
    [DllImport("gdi32.dll")] private static extern bool DeleteDC(nint dc);
    [DllImport("gdi32.dll")] private static extern bool PatBlt(nint dc, int x, int y, int width, int height, uint operation);
    [DllImport("gdi32.dll")] private static extern bool BitBlt(nint target, int x, int y, int width, int height,
        nint source, int sourceX, int sourceY, uint operation);
}
