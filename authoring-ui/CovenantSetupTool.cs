using System.Diagnostics;
using System.Text;

namespace Covenant.Setup.Authoring;

internal sealed record CovenantSetupTool(string Path);

internal sealed record PackageResult(
    bool Succeeded,
    int ExitCode,
    string Output,
    string Error);

internal sealed record ToolValidationResult(
    bool IsValid,
    CovenantSetupTool? Tool,
    string Message);

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

internal static class CovenantSetupToolValidator
{
    public static async Task<ToolValidationResult> ValidateAsync(
        string path,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return Invalid("Pick a valid covenant-setup executable.");
        }

        string fullPath;
        try
        {
            fullPath = System.IO.Path.GetFullPath(path.Trim());
        }
        catch (Exception ex)
        {
            return Invalid("The covenant-setup path is invalid: " + ex.Message);
        }

        if (!File.Exists(fullPath))
        {
            return Invalid("Pick a valid covenant-setup executable.");
        }

        var output = new StringBuilder();
        var error = new StringBuilder();
        using var process = new Process
        {
            StartInfo = CreateHelpStartInfo(fullPath),
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

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(5));

        try
        {
            if (!process.Start())
            {
                return Invalid("Pick a valid covenant-setup executable.");
            }

            process.BeginOutputReadLine();
            process.BeginErrorReadLine();
            await process.WaitForExitAsync(timeout.Token);
            process.WaitForExit();
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            TryKill(process);
            return Invalid("The selected executable did not respond to the covenant-setup help command.");
        }
        catch (Exception ex)
        {
            return Invalid("The selected executable could not be started: " + ex.Message);
        }

        var combinedOutput = string.Join(
            Environment.NewLine,
            new[] { output.ToString(), error.ToString() }.Where(part => !string.IsNullOrWhiteSpace(part)));
        if (process.ExitCode == 0 && LooksLikeCovenantSetupHelp(combinedOutput))
        {
            return new ToolValidationResult(true, new CovenantSetupTool(fullPath), "Packaging enabled: " + fullPath);
        }

        return Invalid("Pick a valid covenant-setup executable.");
    }

    private static ProcessStartInfo CreateHelpStartInfo(string path)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = path,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true
        };
        startInfo.ArgumentList.Add("--help");
        return startInfo;
    }

    private static bool LooksLikeCovenantSetupHelp(string output) =>
        !string.IsNullOrWhiteSpace(output) &&
        (output.Contains("covenant", StringComparison.OrdinalIgnoreCase) ||
            (output.Contains("package", StringComparison.OrdinalIgnoreCase) &&
                output.Contains("install", StringComparison.OrdinalIgnoreCase)));

    private static ToolValidationResult Invalid(string message) =>
        new(false, null, message);

    private static void TryKill(Process process)
    {
        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch
        {
            // Best effort; the caller reports validation failure either way.
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
