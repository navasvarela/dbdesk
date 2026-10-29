// Launches a macOS app and reports, as JSON, how long its first main window took to appear and
// how long until the app went quiet. Run by site/measure.sh through `osascript -l JavaScript`:
//
//   osascript -l JavaScript site/window-probe.js <bundle id> <open arguments…>
//
// JavaScript for Automation is used because it ships with macOS and can call CoreGraphics, whose
// window list gives each window's owner, layer and bounds without the Screen Recording
// permission (only window titles need it). Both times are measured from the same clock, taken
// just before `open` runs.

ObjC.import("AppKit");
ObjC.import("CoreGraphics");

// A splash screen or a first-run dialog is smaller than this; every app measured opens its main
// window larger (DbRover at 1440 × 900 points, DBeaver's workbench and DataGrip's welcome window
// at their defaults), so the first window this big is the one the user works in.
const MAIN_WINDOW_MIN_WIDTH = 800;
const MAIN_WINDOW_MIN_HEIGHT = 500;

// "Quiet" is the app's whole process tree using under 5% of one core for two seconds running,
// sampled every half second. JVM apps go on loading plugins and indexing after their window is
// up; this catches when that stops.
const QUIET_CPU_SHARE = 0.05;
const QUIET_SAMPLES = 4;
const SAMPLE_INTERVAL_S = 0.5;
const POLL_INTERVAL_S = 0.01;
const TIMEOUT_MS = 120000;

function run(argv) {
  const [bundleId, ...openArgs] = argv;
  const app = Application.currentApplication();
  app.includeStandardAdditions = true;

  const webKitBefore = new Set(processRows(app).filter(isWebKit).map((row) => row.pid));
  const start = Date.now();
  app.doShellScript(["open", "-n", "-b", bundleId, ...openArgs].map(quote).join(" "));

  const pid = waitFor(() => runningPid(bundleId), start, "the app to start");
  const windowMs = waitFor(() => (hasMainWindow(pid) ? Date.now() - start : null), start,
    "a main window");
  const quietMs = waitForQuiet(app, pid, webKitBefore, start);
  return JSON.stringify({ pid, window_ms: windowMs, quiet_ms: quietMs });
}

function waitFor(check, start, what) {
  for (;;) {
    const value = check();
    if (value !== null) return value;
    if (Date.now() - start > TIMEOUT_MS) throw new Error(`timed out waiting for ${what}`);
    delay(POLL_INTERVAL_S);
  }
}

function runningPid(bundleId) {
  const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(bundleId);
  return apps.count > 0 ? apps.objectAtIndex(apps.count - 1).processIdentifier : null;
}

function hasMainWindow(pid) {
  // The list is a CFArray, which has to be bridged to an NSArray before it can be unwrapped.
  const windows = ObjC.deepUnwrap(ObjC.castRefToObject(
    $.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, $.kCGNullWindowID)));
  return windows.some((w) => w.kCGWindowOwnerPID === pid
    && w.kCGWindowLayer === 0
    && w.kCGWindowBounds.Width >= MAIN_WINDOW_MIN_WIDTH
    && w.kCGWindowBounds.Height >= MAIN_WINDOW_MIN_HEIGHT);
}

// Returns the time at which the last run of quiet samples began.
function waitForQuiet(app, pid, webKitBefore, start) {
  let previous = appCpuSeconds(app, pid, webKitBefore);
  let previousAt = Date.now();
  let quietSince = null;
  let quietSamples = 0;
  for (;;) {
    delay(SAMPLE_INTERVAL_S);
    const now = Date.now();
    const cpu = appCpuSeconds(app, pid, webKitBefore);
    const share = (cpu - previous) / ((now - previousAt) / 1000);
    if (share < QUIET_CPU_SHARE) {
      if (quietSamples === 0) quietSince = previousAt;
      quietSamples += 1;
      if (quietSamples >= QUIET_SAMPLES) return quietSince - start;
    } else {
      quietSamples = 0;
    }
    previous = cpu;
    previousAt = now;
    if (now - start > TIMEOUT_MS) throw new Error("timed out waiting for the app to go quiet");
  }
}

function processRows(app) {
  return app.doShellScript("ps -A -o pid=,ppid=,time=,comm=").split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/))
    .map(([p, parent, time, ...command]) => ({
      pid: Number(p), ppid: Number(parent), time, command: command.join(" "),
    }));
}

// A WKWebView's page runs in WebKit processes that launchd starts, not as the app's children
// (DBX's whole window is one), so they are counted when they started after the app did.
// site/measure.sh counts memory the same way.
function isWebKit(row) {
  return row.command.includes("com.apple.WebKit.");
}

// CPU time used so far by the process, its descendants and the WebKit processes started since
// it launched, from `ps`'s TIME column ([[hh:]mm:]ss.cc).
function appCpuSeconds(app, pid, webKitBefore) {
  const rows = processRows(app);
  const tree = new Set([pid]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const row of rows) {
      if (!tree.has(row.pid) && tree.has(row.ppid)) {
        tree.add(row.pid);
        grew = true;
      }
    }
  }
  return rows.filter((row) => tree.has(row.pid) || (isWebKit(row) && !webKitBefore.has(row.pid)))
    .reduce((sum, row) => sum + cpuSeconds(row.time), 0);
}

function cpuSeconds(time) {
  return time.split(":").reduce((total, part) => total * 60 + Number(part), 0);
}

function quote(word) {
  return `'${String(word).replace(/'/g, "'\\''")}'`;
}
