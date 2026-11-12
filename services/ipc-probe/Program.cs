using System.Diagnostics;
using System.Globalization;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.Principal;
using Aiwin11.V1.Ipc;
using Grpc.Core;
using Grpc.Net.Client;
using Microsoft.AspNetCore.Connections;
using Microsoft.AspNetCore.Connections.Features;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Win32.SafeHandles;

if (args is ["--worker", var name, var client]
    && Guid.TryParseExact(name, "N", out _)
    && uint.TryParse(client, NumberStyles.None, CultureInfo.InvariantCulture, out var clientPid)
    && clientPid != 0)
{
    await ServeAsync($"aiwin11-ipc-{name}", clientPid);
    return;
}
if (args.Length != 0) throw new ArgumentException("Unexpected arguments.");

await CheckAsync((uint)Environment.ProcessId, expectAccepted: true);
await CheckAsync(uint.MaxValue, expectAccepted: false);
Console.WriteLine("gRPC IPC: authenticated worker, schema validation, wrong client and wrong server PID checks passed.");

static async Task ServeAsync(string pipeName, uint clientPid)
{
    var builder = WebApplication.CreateSlimBuilder(new WebApplicationOptions { Args = [] });
    builder.Configuration.Sources.Clear();
    builder.Logging.ClearProviders();
    builder.WebHost.UseNamedPipes(options => options.CurrentUserOnly = true);
    builder.WebHost.ConfigureKestrel(options => options.ListenNamedPipe(pipeName, endpoint =>
    {
        endpoint.Protocols = HttpProtocols.Http2;
        endpoint.Use(next => async connection =>
        {
            var pipe = connection.Features.Get<IConnectionNamedPipeFeature>()?.NamedPipe;
            if (pipe is null || !Native.GetNamedPipeClientProcessId(pipe.SafePipeHandle, out uint pid)
                || pid != clientPid)
            {
                connection.Abort();
                return;
            }
            await next(connection);
        });
    }));
    builder.Services.AddGrpc(options => options.MaxReceiveMessageSize = 64);
    await using var app = builder.Build();
    app.MapGrpcService<ProbeService>();
    await app.StartAsync();
    using var parent = Console.OpenStandardInput();
    await parent.ReadAtLeastAsync(new byte[1], 1, false);
    using var stop = new CancellationTokenSource(TimeSpan.FromSeconds(2));
    await app.StopAsync(stop.Token);
}

static async Task CheckAsync(uint expectedClient, bool expectAccepted)
{
    var name = Guid.NewGuid().ToString("N");
    var start = new ProcessStartInfo(Environment.ProcessPath!) { RedirectStandardInput = true };
    // dotnet run uses the apphost. Also support "dotnet AiWin11.IpcProbe.dll".
    if (Path.GetFileNameWithoutExtension(start.FileName).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
        start.ArgumentList.Add(typeof(ProbeService).Assembly.Location);
    start.ArgumentList.Add("--worker");
    start.ArgumentList.Add(name);
    start.ArgumentList.Add(expectedClient.ToString(CultureInfo.InvariantCulture));
    using var worker = Process.Start(start) ?? throw new InvalidOperationException("Worker failed to start.");
    try
    {
        uint workerPid = (uint)worker.Id;
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        using var channel = Connect($"aiwin11-ipc-{name}", workerPid);
        var probe = new IpcProbe.IpcProbeClient(channel);
        if (!expectAccepted)
        {
            await ExpectAsync(StatusCode.Unavailable, () => CallAsync(probe, 1, timeout.Token));
            return;
        }
        var result = await CallAsync(probe, 1, timeout.Token);
        if (result.SchemaVersion != 1 || result.WorkerProcessId != workerPid)
            throw new InvalidOperationException("Unexpected worker reply.");
        await ExpectAsync(StatusCode.InvalidArgument, () => CallAsync(probe, 2, timeout.Token));
        using var wrongServer = Connect($"aiwin11-ipc-{name}", uint.MaxValue);
        await ExpectAsync(StatusCode.Unavailable,
            () => CallAsync(new IpcProbe.IpcProbeClient(wrongServer), 1, timeout.Token));
    }
    finally
    {
        worker.StandardInput.Close();
        if (!worker.WaitForExit(4000))
        {
            worker.Kill();
            await worker.WaitForExitAsync();
            throw new InvalidOperationException("Worker did not exit after the parent closed stdin.");
        }
        if (worker.ExitCode != 0) throw new InvalidOperationException($"Worker exited with {worker.ExitCode}.");
    }
}

static GrpcChannel Connect(string pipeName, uint expectedServer) =>
    GrpcChannel.ForAddress("http://localhost", new GrpcChannelOptions
    {
        HttpHandler = new SocketsHttpHandler
        {
            UseProxy = false,
            ConnectCallback = async (_, cancellation) =>
            {
                var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut,
                    PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly, TokenImpersonationLevel.Anonymous);
                try
                {
                    await pipe.ConnectAsync(5000, cancellation);
                    if (!Native.GetNamedPipeServerProcessId(pipe.SafePipeHandle, out uint pid) || pid != expectedServer)
                        throw new IOException("Unexpected pipe server process.");
                    return pipe;
                }
                catch
                {
                    await pipe.DisposeAsync();
                    throw;
                }
            },
        },
    });

static async Task<ProbeReply> CallAsync(IpcProbe.IpcProbeClient probe, uint version, CancellationToken cancellation) =>
    await probe.CheckAsync(new ProbeRequest { SchemaVersion = version }, cancellationToken: cancellation);

static async Task ExpectAsync(StatusCode expected, Func<Task<ProbeReply>> call)
{
    try { await call(); }
    catch (RpcException error) when (error.StatusCode == expected) { return; }
    throw new InvalidOperationException($"Expected gRPC status {expected}.");
}

internal sealed class ProbeService : IpcProbe.IpcProbeBase
{
    public override Task<ProbeReply> Check(ProbeRequest request, ServerCallContext context) =>
        request.SchemaVersion == 1
            ? Task.FromResult(new ProbeReply { SchemaVersion = 1, WorkerProcessId = (uint)Environment.ProcessId })
            : throw new RpcException(new Status(StatusCode.InvalidArgument, "Unsupported IPC schema version."));
}

internal static class Native
{
    [DllImport("kernel32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool GetNamedPipeClientProcessId(SafePipeHandle pipe, out uint processId);

    [DllImport("kernel32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out uint processId);
}
