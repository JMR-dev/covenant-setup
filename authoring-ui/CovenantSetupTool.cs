using System.Diagnostics;
using System.Text;

namespace Covenant.Setup.Authoring;

internal sealed record CovenantSetupTool(string Path);

internal sealed record PackageResult(
    bool Succeeded,
    int ExitCode,
    string Output,
    string Error);

internal static class CovenantSetupToolLocator
{
    public static CovenantSetupTool? Find()
    {
        return Find(CandidateDirectories());
    }

    internal static CovenantSetupTool? Find(IEnumerable<string> candidateDirectories)
    {
        foreach (var candidate in CandidatePaths(candidateDirectories))
        {
            if (File.Exists(candidate))
            {
                return new CovenantSetupTool(candidate);
            }
        }

        return null;
    }

    private static IEnumerable<string> CandidatePaths(IEnumerable<string> candidateDirectories)
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var directory in candidateDirectories)
        {
            var candidate = System.IO.Path.Combine(directory, "covenant-setup.exe");
            if (seen.Add(candidate))
            {
                yield return candidate;
            }
        }
    }

    private static IEnumerable<string> CandidateDirectories()
    {
        yield return AppContext.BaseDirectory;
        yield return Directory.GetCurrentDirectory();

        foreach (var root in Ancestors(Directory.GetCurrentDirectory()).Concat(Ancestors(AppContext.BaseDirectory)))
        {
            yield return System.IO.Path.Combine(root, "target", "release");
            yield return System.IO.Path.Combine(root, "target", "debug");
        }

        var path = Environment.GetEnvironmentVariable("PATH");
        if (string.IsNullOrWhiteSpace(path))
        {
            yield break;
        }

        foreach (var entry in path.Split(System.IO.Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
        {
            var trimmed = entry.Trim();
            if (!string.IsNullOrWhiteSpace(trimmed))
            {
                yield return trimmed;
            }
        }
    }

    private static IEnumerable<string> Ancestors(string start)
    {
        var current = new DirectoryInfo(start);
        while (current is not null)
        {
            yield return current.FullName;
            current = current.Parent;
        }
    }
}

internal static class CovenantSetupPackager
{
    public static async Task<PackageResult> PackageAsync(
        CovenantSetupTool tool,
        string manifestPath,
        string outputDirectory,
        CancellationToken cancellationToken)
    {
        Directory.CreateDirectory(outputDirectory);

        var output = new StringBuilder();
        var error = new StringBuilder();
        using var process = new Process
        {
            StartInfo = CreateStartInfo(tool, manifestPath, outputDirectory),
            EnableRaisingEvents = true
        };

        process.OutputDataReceived += (_, args) =>
        {
            if (args.Data is not null)
            {
                output.AppendLine(args.Data);
            }
        };
        process.ErrorDataReceived += (_, args) =>
        {
            if (args.Data is not null)
            {
                error.AppendLine(args.Data);
            }
        };

        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        await process.WaitForExitAsync(cancellationToken);
        return new PackageResult(
            process.ExitCode == 0,
            process.ExitCode,
            output.ToString().Trim(),
            error.ToString().Trim());
    }

    internal static ProcessStartInfo CreateStartInfo(
        CovenantSetupTool tool,
        string manifestPath,
        string outputDirectory)
    {
        var manifestDirectory = System.IO.Path.GetDirectoryName(manifestPath);
        var startInfo = new ProcessStartInfo
        {
            FileName = tool.Path,
            WorkingDirectory = string.IsNullOrWhiteSpace(manifestDirectory)
                ? Environment.CurrentDirectory
                : manifestDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true
        };
        startInfo.ArgumentList.Add("--json");
        startInfo.ArgumentList.Add("package");
        startInfo.ArgumentList.Add(manifestPath);
        startInfo.ArgumentList.Add("--output");
        startInfo.ArgumentList.Add(outputDirectory);
        return startInfo;
    }
}
