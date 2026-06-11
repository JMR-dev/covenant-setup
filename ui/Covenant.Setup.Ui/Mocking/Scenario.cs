using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;

namespace Covenant.Setup.Ui.Mocking;

internal abstract record ScenarioStep;
internal sealed record SendStep(string JsonLine) : ScenarioStep;
internal sealed record DelayStep(TimeSpan Delay) : ScenarioStep;
internal sealed record AwaitResponseStep(string Id, string? Expect) : ScenarioStep;

internal sealed class Scenario(string name, IReadOnlyList<ScenarioStep> steps)
{
    public string Name => name;
    public IReadOnlyList<ScenarioStep> Steps => steps;

    public static Scenario Parse(string name, IEnumerable<string> lines)
    {
        var steps = new List<ScenarioStep>();
        int lineNumber = 0;
        foreach (var rawLine in lines)
        {
            lineNumber++;
            var trimmed = rawLine.Trim();
            if (string.IsNullOrEmpty(trimmed) || trimmed.StartsWith('#'))
            {
                continue;
            }

            try
            {
                using var doc = JsonDocument.Parse(trimmed);
                var root = doc.RootElement;
                if (root.ValueKind != JsonValueKind.Object)
                {
                    throw new ScenarioFormatException($"Line {lineNumber}: Root element must be a JSON object.");
                }

                if (root.TryGetProperty("type", out _))
                {
                    steps.Add(new SendStep(trimmed));
                }
                else if (root.TryGetProperty("wait_ms", out var waitMsProp))
                {
                    if (waitMsProp.ValueKind == JsonValueKind.Number)
                    {
                        var ms = waitMsProp.GetDouble();
                        steps.Add(new DelayStep(TimeSpan.FromMilliseconds(ms)));
                    }
                    else
                    {
                        throw new ScenarioFormatException($"Line {lineNumber}: wait_ms must be a number.");
                    }
                }
                else if (root.TryGetProperty("await_response", out var awaitProp))
                {
                    if (awaitProp.ValueKind == JsonValueKind.Object)
                    {
                        if (awaitProp.TryGetProperty("id", out var idProp) && idProp.ValueKind == JsonValueKind.String)
                        {
                            var id = idProp.GetString()!;
                            string? expect = null;
                            if (awaitProp.TryGetProperty("expect", out var expectProp))
                            {
                                if (expectProp.ValueKind == JsonValueKind.String)
                                {
                                    expect = expectProp.GetString();
                                }
                                else if (expectProp.ValueKind != JsonValueKind.Null)
                                {
                                    throw new ScenarioFormatException($"Line {lineNumber}: await_response.expect must be a string or null.");
                                }
                            }
                            steps.Add(new AwaitResponseStep(id, expect));
                        }
                        else
                        {
                            throw new ScenarioFormatException($"Line {lineNumber}: await_response must contain a string 'id'.");
                        }
                    }
                    else
                    {
                        throw new ScenarioFormatException($"Line {lineNumber}: await_response must be an object.");
                    }
                }
                else
                {
                    throw new ScenarioFormatException($"Line {lineNumber}: Unknown step shape. Must contain 'type', 'wait_ms', or 'await_response'.");
                }
            }
            catch (JsonException ex)
            {
                throw new ScenarioFormatException($"Line {lineNumber}: Invalid JSON - {ex.Message}");
            }
        }

        return new Scenario(name, steps);
    }

    public static Scenario LoadFile(string path)
    {
        var resolved = ResolvePath(path);
        var lines = File.ReadLines(resolved);
        return Parse(Path.GetFileNameWithoutExtension(resolved), lines);
    }

    public static string ResolvePath(string nameOrPath)
    {
        if (string.IsNullOrWhiteSpace(nameOrPath))
        {
            throw new ArgumentException("Name or path cannot be null or empty.", nameof(nameOrPath));
        }

        if (!nameOrPath.Contains(Path.DirectorySeparatorChar) && !nameOrPath.Contains(Path.AltDirectorySeparatorChar))
        {
            var name = nameOrPath;
            if (!name.EndsWith(".jsonl", StringComparison.OrdinalIgnoreCase))
            {
                name += ".jsonl";
            }
            return Path.Combine(AppContext.BaseDirectory, "Scenarios", name);
        }

        return Path.GetFullPath(nameOrPath);
    }
}

internal sealed class ScenarioFormatException(string message) : Exception(message);
