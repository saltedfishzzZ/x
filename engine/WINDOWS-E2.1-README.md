# E2.1 Windows native validation

1. Extract the complete archive to a local disk.
2. Ensure a compatible Rust toolchain and Python 3 are installed.
3. Run `run-e2.1-windows.cmd`.
4. Wait for `E2.1 local candidate checks: PASS`.
5. Copy `reports/e2.1/platform-windows-amd64.json` back to the same directory in the macOS workspace.

The verifier uses locked, offline Cargo dependencies. G18 through G20 remain `NOT_EVALUATED` in each candidate report until cross-platform comparison and final freeze.
