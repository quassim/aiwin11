using System.Runtime.InteropServices;
using System.Windows.Automation;
using Protocol = Aiwin11.V1;

internal static class UiInspector
{
    private const int MaxNodes = 40;
    private const int MaxDepth = 6;

    internal static Protocol.UiTree? Read(long deadline)
    {
        nint window = GetForegroundWindow();
        if (window == 0 || GetWindowThreadProcessId(window, out uint pid) == 0 || pid == 0)
            return null;

        var root = AutomationElement.FromHandle(window);
        var tree = new Protocol.UiTree();
        Walk(root, 0, 0);
        return tree.Nodes.Count > 0 && GetForegroundWindow() == window ? tree : null;

        void Walk(AutomationElement element, uint parent, int depth)
        {
            if (tree.Nodes.Count == MaxNodes || depth > MaxDepth
                || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline)
            {
                tree.Truncated = true;
                return;
            }
            var current = element.Current;
            if (current.ProcessId != pid)
                return;

            tree.Nodes.Add(new Protocol.UiNode
            {
                Parent = parent,
                ControlType = (uint)current.ControlType.Id,
                Enabled = current.IsEnabled,
                Focused = current.HasKeyboardFocus,
            });
            uint index = (uint)tree.Nodes.Count;
            var walker = TreeWalker.ControlViewWalker;
            for (var child = walker.GetFirstChild(element); child is not null; child = walker.GetNextSibling(child))
            {
                Walk(child, index, depth + 1);
                if (tree.Nodes.Count == MaxNodes || DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() >= deadline)
                {
                    tree.Truncated = true;
                    break;
                }
                if (depth == MaxDepth)
                    break;
            }
        }
    }

    [DllImport("user32.dll")]
    private static extern nint GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(nint window, out uint processId);
}
