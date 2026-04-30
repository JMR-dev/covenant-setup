using System.Text;

namespace Covenant.Setup.Authoring;

internal static class ManifestTomlWriter
{
    public static string Write(ManifestDocument document)
    {
        var builder = new StringBuilder();
        AppendAssignment(builder, "app_name", document.AppName);

        foreach (var directory in document.Directories)
        {
            AppendBlankLine(builder);
            builder.AppendLine("[[directories]]");
            AppendAssignment(builder, "path", directory.Path);
        }

        foreach (var file in document.Files)
        {
            AppendBlankLine(builder);
            builder.AppendLine("[[files]]");
            AppendAssignment(builder, "source", file.Source);
            AppendAssignment(builder, "destination", file.Destination);
        }

        foreach (var registry in document.Registry)
        {
            AppendBlankLine(builder);
            builder.AppendLine("[[registry]]");
            AppendAssignment(builder, "key", registry.Key);
            AppendAssignment(builder, "name", registry.Name);
            AppendAssignment(builder, "value", registry.Value);
        }

        foreach (var shortcut in document.Shortcuts)
        {
            AppendBlankLine(builder);
            builder.AppendLine("[[shortcuts]]");
            AppendAssignment(builder, "path", shortcut.Path);
            AppendAssignment(builder, "target", shortcut.Target);
            AppendOptionalAssignment(builder, "arguments", shortcut.Arguments);
            AppendOptionalAssignment(builder, "working_directory", shortcut.WorkingDirectory);
            AppendOptionalAssignment(builder, "description", shortcut.Description);
        }

        foreach (var script in document.Scripts)
        {
            AppendBlankLine(builder);
            builder.AppendLine("[[scripts]]");
            AppendAssignment(builder, "command", script.Command);
            AppendArray(builder, "args", script.Args);
            AppendOptionalAssignment(builder, "working_directory", script.WorkingDirectory);
        }

        AppendBlankLine(builder);
        builder.AppendLine("[purge]");
        AppendInlineArray(builder, "registry_branches", document.Purge.RegistryBranches);
        AppendInlineArray(builder, "paths", document.Purge.Paths);

        return builder.ToString();
    }

    private static void AppendAssignment(StringBuilder builder, string key, string value)
    {
        builder.Append(key).Append(" = ").Append(TomlString(value)).AppendLine();
    }

    private static void AppendOptionalAssignment(StringBuilder builder, string key, string? value)
    {
        if (!string.IsNullOrWhiteSpace(value))
        {
            AppendAssignment(builder, key, value.Trim());
        }
    }

    private static void AppendArray(StringBuilder builder, string key, IReadOnlyList<string> values)
    {
        if (values.Count == 0)
        {
            builder.Append(key).AppendLine(" = []");
            return;
        }

        builder.Append(key).AppendLine(" = [");
        for (var index = 0; index < values.Count; index++)
        {
            builder.Append("  ").Append(TomlString(values[index]));
            if (index < values.Count - 1)
            {
                builder.Append(',');
            }
            builder.AppendLine();
        }
        builder.AppendLine("]");
    }

    private static void AppendInlineArray(StringBuilder builder, string key, IReadOnlyList<string> values)
    {
        builder.Append(key).Append(" = [");
        for (var index = 0; index < values.Count; index++)
        {
            if (index > 0)
            {
                builder.Append(", ");
            }
            builder.Append(TomlString(values[index]));
        }
        builder.AppendLine("]");
    }

    private static void AppendBlankLine(StringBuilder builder)
    {
        if (builder.Length > 0)
        {
            builder.AppendLine();
        }
    }

    private static string TomlString(string value)
    {
        var builder = new StringBuilder(value.Length + 2);
        builder.Append('"');
        foreach (var ch in value)
        {
            switch (ch)
            {
                case '\\':
                    builder.Append(@"\\");
                    break;
                case '"':
                    builder.Append("\\\"");
                    break;
                case '\n':
                    builder.Append(@"\n");
                    break;
                case '\r':
                    builder.Append(@"\r");
                    break;
                case '\t':
                    builder.Append(@"\t");
                    break;
                default:
                    if (char.IsControl(ch))
                    {
                        builder.Append(@"\u").Append(((int)ch).ToString("x4"));
                    }
                    else
                    {
                        builder.Append(ch);
                    }
                    break;
            }
        }
        builder.Append('"');
        return builder.ToString();
    }
}
