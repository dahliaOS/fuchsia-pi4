Bluetooth
=========

The Fuchsia Bluetooth system aims to provide a dual-mode implementation of the
Bluetooth Host Subsystem (5.0+) supporting a framework for developing Low Energy
and Traditional profiles.

Source code shortcuts:
- Public API:
  * [shared](/sdk/fidl/fuchsia.bluetooth)
  * [System API](/sdk/fidl/fuchsia.bluetooth.sys)
  * [BR/EDR (Profile)](/sdk/fidl/fuchsia.bluetooth.bredr)
  * [GATT](/sdk/fidl/fuchsia.bluetooth.gatt)
  * [LE](/sdk/fidl/fuchsia.bluetooth.le)
- [Private API](/src/connectivity/bluetooth/fidl)
- [Tools](tools/)
- [Host Subsystem Driver](core/bt-host)
- [HCI Drivers](hci)
- [HCI Transport Drivers](hci/transport)

For more orientation, see
- [System Architecture](/docs/concepts/bluetooth/architecture.md)
- [Respectful Code](#Respectful-Code)

For a note on used (and avoided) vocabulary, see
- [Bluetooth Vocabulary](docs/vocabulary.md)

## Getting Started

### API Examples

Examples using Fuchsia's Bluetooth Low Energy APIs can be found
[here](examples).

### Privileged System API

Dual-mode (LE + Classic) GAP operations that are typically exposed to privileged
clients are performed using the [fuchsia.bluetooth.sys](/sdk/fidl/fuchsia.bluetooth.sys) library.
This API is intended for managing local adapters, device discovery & discoverability,
pairing/bonding, and other settings.

[`bt-cli`](tools/bt-cli) is a command-line front-end for privileged access operations:

```
$ bt-cli
bt> list-adapters
Adapter:
    Identifier:     e5878e9f642d8908
    Address:        34:13:E8:86:8C:19
    Technology:     DualMode
    Local Name:     siren-relic-wad-pout
    Discoverable:   false
    Discovering:    false
    Local UUIDs:    None
```

**NOTE**: _fuchsia.bluetooth.sys replaces the deprecated
[fuchsia.bluetooth.control](/sdk/fidl/fuchsia.bluetooth.control) API, which contiues to be
supported. The bt-cli tool currently uses the deprecated API._

### Tools

See the [bluetooth/tools](tools/) package for more information on
available command line tools for testing/debugging.

### Running Tests

Your build configuration may or may not include Bluetooth tests. Ensure
Bluetooth tests are built and installed when paving or OTA'ing with [`fx set`](docs/development/build/fx.md#configure-a-build):

  ```
  $ fx set workstation.x64 --with-base="//bundles:tools,//src/connectivity/bluetooth"
  ```

#### Tests

In general, the Bluetooth codebase defines an associated unit test binary for each production
binary and library, as well as a number of integration test binaries. Look in the GN file of a
production binary or library to find its associated unit tests.

Each test binary is a [component](/docs/glossary.md#component)
whose runtime environment is defined by its [`.cmx` component manifest](/docs/the-book/package_metadata.md#Component-Manifest)

For example, `bt-host-l2cap-tests` is a [Google Test](https://github.com/google/googletest)
binary that contains all the C++ L2CAP subsystem unit tests and is a standalone test package.

##### Running on a Fuchsia device

* Run all the bt-host unit tests:

  ```
  $ fx test //src/connectivity/bluetooth/core/bt-host
  ```

To see all options for running these tests, run `fx test --help`.

##### Running on QEMU

If you don't have physical hardware available, you can run the tests in FEMU using the same commands as above. See [FEMU set up instructions](https://fuchsia.dev/fuchsia-src/get-started/set_up_femu).

#### Integration Tests

See the [Integration Test README](tests/integration/README.md)

### Controlling Log Verbosity

#### Logging in Drivers

The most reliable way to enable higher log verbosity is with kernel command line parameters. These can be configured through the `fx set` command:

  ```
  fx set workstation.x64 --args="dev_bootfs_labels=[\"//src/connectivity/bluetooth:driver-debug-logging\"]"
  ```

This will enable debug-level logging for all supported chipsets.
Using `fx set` writes these values into the image, so they will survive a restart.
For more detail on driver logging, see [Zircon driver logging](/docs/concepts/drivers/driver-development.md#logging)

#### Profile Level Logging
Each Bluetooth profile has logging that can be turned on and can be useful during debugging.
They're fully documented in the [profile-specific README's here](/src/connectivity/bluetooth/profiles/README.md) but there are a couple of examples below.

  ```
  a2dp-sink=trace
  a2dp-source=trace
  ```

#### bin/bt-gap

The Bluetooth system service is invoked by sysmgr to resolve service requests.
The mapping between environment service names and their handlers is defined in
[//src/sys/sysmgr/config/services.config](/src/sys/sysmgr/config/services.config).
Add the `--verbose` option to the Bluetooth entries to increase verbosity, for
example:

  ```
  ...
    "fuchsia.bluetooth.bredr.Profile":  "fuchsia-pkg://fuchsia.com/bt-init#meta/bt-init.cmx",
    "fuchsia.bluetooth.control.Control": "fuchsia-pkg://fuchsia.com/bt-init#meta/bt-init.cmx",
    "fuchsia.bluetooth.gatt.Server":  "fuchsia-pkg://fuchsia.com/bt-init#meta/bt-init.cmx",
    "fuchsia.bluetooth.le.Central":  "fuchsia-pkg://fuchsia.com/bt-init#meta/bt-init.cmx",
    "fuchsia.bluetooth.le.Peripheral":  "fuchsia-pkg://fuchsia.com/bt-init#meta/bt-init.cmx",
    "fuchsia.bluetooth.snoop.Snoop":  "fuchsia-pkg://fuchsia.com/bt-snoop#meta/bt-snoop.cmx",
  ...

  ```

### Inspecting Component State

The Bluetooth system supports inspection through the [Inspect API](/docs/development/diagnostics/inspect).
bt-gap, bt-host, bt-a2dp-sink, and bt-snoop all expose information though Inspect.

#### Usage

* bt-host: `fx iquery show-file /dev/diagnostics/class/bt-host/000.inspect` exposes information about the controller, peers, and services.
* bt-gap: `fx iquery show bt-gap` exposes information on host devices managed by bt-gap, pairing capabilities, stored bonds, and actively connected peers.
* bt-a2dp-sink: `fx iquery show bt-a2dp-sink` exposes information on audio streaming capabilities and active streams
* bt-snoop: `fx iquery show bt-snoop` exposes information about which hci devices are being logged and how much data is stored.
* All Bluetooth components: `fx iquery show bt-*`

See the [iquery documentation](/docs/development/diagnostics/inspect/iquery) for complete instructions on using `iquery`.

### Respectful Code

Inclusivity is central to Fuchsia's culture, and our values include treating
each other with dignity. As such, it???s important that everyone can contribute
without facing the harmful effects of bias and discrimination.

The Bluetooth standard makes use of the terms "master" and "slave" to define
link layer connection roles in many of the protocol specifications. Here are a
few rules of thumb when referring to these roles in code and comments:

1. Do not propagate these terms beyond the layer of code directly involved with link layer
roles. Use the suggested alternative terminology at FIDL API boundaries. See
[Bluetooth Vocabulary Guide](//src/connectivity/bluetooth/docs/vocabulary.md).

2. Whenever possible, prefer different terms that more specifically describe function. For example,
the SMP specification defines "initiator" and "responder" roles that correspond to the
aforementioned roles without loss of clarity.

3. If an explicit reference to the link layer role is necessary, then try to
avoid the term "slave" where possible. For example this formulation avoids the
term without losing clarity:

```
   if (link->role() != hci::Connection::Role::kMaster) {
     ...
   }
```

See the Fuchsia project [guide](//docs/best-practices/respectful_code.md) on best practices
for more information.
