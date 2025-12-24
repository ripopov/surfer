# Remote file access

It is possible to run a remote Surfer server, called Surver, on a different computer and then connect to that computer to avoid downloading/copying large waveform files. Only the requested information will be transferred.

The recommended way is to use an ssh tunnel to secure the connection. It is described how to set this up when you start Surver.

> [!WARNING]
> If an ssh tunnel is not used, this is done using plain http, so absolutely no security is provided for the access or transmission, apart from having to provide the correct token, which is also transmitted using plain http.

## Surver

There is a stand-alone binary, *Surver*, that can be compiled, resulting in a much smaller binary and more likely to succeed on systems where Surfer may be hard to install due to GUI dependencies not being installed etc.

> [!tip]
> There is basically no point in running `surfer server`, as running `surver` does not have any drawbacks. Most likely, support for running Surfer in server mode will be removed and only Surver can be used.</mark>

## Using remote mode

There are two ways to start the server, either start the stand-alone server binary *Surver*:

``` bash
surver <FILENAME>
```

or start Surfer in server mode using:

``` bash
surfer server --filename <FILENAME>
```

In both situations, instructions how to progress will be printed. There are basically two ways to connect:

1. If the computer running the server is directly accessible, it can be accessed using the provided URL.
2. If not, you will need to setup an SSH tunnel by following the instructions.

Now, Surfer can be started using the provided URL/start command, or you can use File -> Open URL and enter the provided URL.

## Configuration

Currently, the configuration options are quite rudimentary and can be provided on the command line. To see the available configuration values, either execute:

``` bash
surver --help
```

leading to

``` text
Server for the Surfer waveform viewer

Usage: surver [OPTIONS] <WAVE_FILES|--file <FILE>>

Arguments:
  [WAVE_FILES]...  Waveform files in VCD, FST, or GHW format

Options:
      --file <FILE>                  File with one wave form file name per line
      --port <PORT>                  Port on which server will listen
      --bind-address <BIND_ADDRESS>  IP address to bind the server to
      --token <TOKEN>                Token used by the client to authenticate to the server
  -h, --help                         Print help
  -V, --version                      Print version
```

or

``` bash
surfer server --help
```

which will print a similar set of options.

> [!caution]
> Using `--bind-address` and accessing the connection directly is not secure. This should only be done in a local network.
