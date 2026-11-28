# 0038. WSL distributions as devices

- Status: Accepted
- Date: 2026-10-28
- Code: `hide-node/src/ssh/wsl.rs`

## Decision

### WSL distributions as devices

On Windows, each WSL 2 distribution of this PC is a device of its own, reached through `wsl.exe` instead of SSH (PRD wsl-device).
A registration stores the distribution's name (`wsl_distro`, `LinkOrigin::Wsl`) instead of an SSH alias; its id is `wsl-<name>-<digest>`: the name lowercased with any letter but an ASCII letter, a digit or `-` written as `-`, and the first eight hex digits of the SHA-256 of the exact name, so the digest, not the readable part, tells two names apart; its name is `WSL <name>`; both are the core's, so the Add device dialog shows the name and has nothing to type.
A build before this field reads a WSL record as a registration with neither an alias nor a distribution and drops that record alone; its SSH devices stay (`persistence.rs`'s downgrade test).
The core reaches it through the same `DeviceConnector` (`wsl_transport`, `wsl_distributions`) and `DialedTransport::establish` as an SSH device, and `hide-node/src/ssh/wsl.rs` is the transport.
Its order refuses before it writes anything: a WSL 1 distribution or one whose first-run setup is pending is `Unsupported`, saying what fixes it (`wsl --set-version <name> 2`, or finishing the setup); a stopped distribution on an attempt that may not start it is `Stopped`, after one running check and no other call; then one `wsl.exe -d <name> --cd ~ -e sh -c …` runs the same start script as an SSH device ([A device's ssh](0037-device-ssh.md#a-devices-ssh)), whose probe reads the user, uid, home, `uname` and `sha256sum`, and the identity (the user, the distribution as host name, port 0, and `wsl:<distribution id>` as machine id) must match the consent, or it is `IdentityChanged` and asked again; a consent from before the machine id, which named `wsl:<distribution id>` as its host key, still matches.
The install sends only what the version folder lacks or holds damaged, each file a counted run of bytes the script checks against its digest and renames into place, so a cut connection leaves no half file and the next one resumes.
The script then execs `<helper> node serve` on the same child, its stdin and stdout the one link (`hide-node/src/ssh/bootstrap.rs`); the distribution's panes run in holders the node starts there, and the kit runs once on connection.
The link owns the child: its end closes the child's stdin, waits five seconds, then ends its process tree, and the child's exit ends the link; the pane holders outlive both.
The five seconds start when the link ends, not when its input is free: a write blocked on a child that stopped reading holds the input, the input is closed once no write holds it, and ending the tree is what frees that write.
The core ends every device's link as it stops (`Core`'s drop, after the coordinators that could dial one again), so quitting Hide ends each distribution's helper; on Windows the job object also ends `wsl.exe` when hided exits without a destructor, and the helper then reads the end of its input, which the WSL lane proves by killing a process that holds a link.
At most four `wsl.exe` calls run at once across every distribution; each has its own deadline (10 s for the running check, 60 s for the probe, 180 s for an install or kit pass).

Starting a stopped distribution is a decision the core makes once per attempt (`HostAttempt` in `runtime/hosts.rs`): the operator opening its tile (`focus_device`, `wake_opened_device`), their own reconnect (`device_host_retry`), adding it and its switch may start it (`Wake::Allowed`), and the app opening, every background retry, a link that ended and an automatic kit or project pass may not (`Wake::NotAllowed`; PRD device-connect-iroh B43, which narrows wsl-device B3: the app opening used to start it).
A `Stopped` answer leaves the device unavailable with no retry scheduled, the row's "not connected", until the operator asks again; a tile opened while such an attempt is still running is kept in `device_wake_pending` and taken by the next attempt.
An SSH device takes the same argument for what its sign-in may ask, which the app opening still allows.
A distribution with a Hide of its own follows the same rule on the paired connection: the attempt's leave reaches hided's dial (`PeerDial::open`), which marks the distribution as one the operator opened for 60 s (`Carriers::may_wake`), asks for a dial now and waits up to 60 s for it, since WSL's virtual machine starts cold; with no leave a carrier is refused for a distribution `wsl.exe --list --running` does not list, so the connection's own redial, at most once a minute, starts nothing.
Add device's listing reads `wsl_distributions` beside the SSH hosts on the same worker; a WSL 1 distribution or one whose setup is pending is listed with why it cannot be chosen, and an unreadable registry is `wsl_problem`, never an empty list.
A WSL device's ports and disk are read through its node, as a node that dialed in reports its own (`reports_own_facts`), and its pull requests with this PC's GitHub login (D-18); its agent hooks and settings are written in the distribution's home only.
