using System.Text.Json;
using Covenant.Setup.Ui;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class ProgramTests
{
    [Fact]
    public void ReadPipeName_returns_value_following_pipe_flag()
    {
        var name = Program.ReadPipeName(new[] { "--pipe", @"\\.\pipe\foo" });
        Assert.Equal(@"\\.\pipe\foo", name);
    }

    [Fact]
    public void ReadPipeName_is_case_insensitive_on_flag()
    {
        var name = Program.ReadPipeName(new[] { "--PIPE", "abc" });
        Assert.Equal("abc", name);
    }

    [Fact]
    public void ReadPipeName_finds_flag_among_other_args()
    {
        var name = Program.ReadPipeName(new[] { "--other", "x", "--pipe", "p1", "--more", "y" });
        Assert.Equal("p1", name);
    }

    [Fact]
    public void ReadPipeName_returns_null_when_flag_missing()
    {
        Assert.Null(Program.ReadPipeName(new[] { "--other", "x" }));
    }

    [Fact]
    public void ReadPipeName_returns_null_when_flag_is_last_arg_with_no_value()
    {
        Assert.Null(Program.ReadPipeName(new[] { "--pipe" }));
    }

    [Fact]
    public void ReadPipeName_returns_null_for_empty_args()
    {
        Assert.Null(Program.ReadPipeName(Array.Empty<string>()));
    }
}
