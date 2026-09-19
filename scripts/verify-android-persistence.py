#!/usr/bin/env python3
"""Run lifecycle and real process-death QA on an explicitly named disposable emulator.

Install the current debug app and Android-test APK first. This utility refuses
physical-device serials and validates the AVD name before stopping the test app.
"""
import argparse
import os
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--adb", required=True)
parser.add_argument("--serial", required=True)
parser.add_argument("--avd-name", required=True)
args = parser.parse_args()
if not args.serial.startswith("emulator-"):
    parser.error("Only a disposable emulator may be used")
base = [args.adb, "-P", os.environ.get("ANDROID_ADB_SERVER_PORT", "5037"), "-s", args.serial]
def run(*arguments):
    process = subprocess.run(base + list(arguments), check=False, capture_output=True, text=True, timeout=60)
    print(process.stdout, end="", flush=True)
    if process.returncode: raise SystemExit(process.stderr or process.stdout)
    return process.stdout
if run("emu", "avd", "name").splitlines()[0].strip() != args.avd_name:
    parser.error("The emulator name does not match the disposable QA target")
result = run("shell", "am", "instrument", "-w", "-e", "class", "com.lawpdf.mobile.PersistenceInstrumentationTest",
             "com.lawpdf.mobile.test/com.lawpdf.mobile.ProcessDeathInstrumentation")
if "OK (4 tests)" not in result:
    raise SystemExit("Lifecycle instrumentation failed")
runner = "com.lawpdf.mobile.test/com.lawpdf.mobile.ProcessDeathInstrumentation"
for phase in ["prepare", "restore"]:
    if phase == "restore":
        run("shell", "am", "force-stop", "com.lawpdf.mobile")
    result = run("shell", "am", "instrument", "-w", "-e", "phase", phase, runner)
    if "LAWPDF_PROCESS_DEATH_OK " + phase not in result or "LAWPDF_PROCESS_DEATH_FAILED" in result:
        raise SystemExit("Process-death phase failed: " + phase)
print("Android lifecycle and force-stop restoration passed")
