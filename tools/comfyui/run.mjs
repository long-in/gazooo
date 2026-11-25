// OS を判定して setup / start のスクリプトを呼び出すディスパッチャ(docs/08-local-comfyui.md)。
// 使い方: node tools/comfyui/run.mjs <setup|start>
import { spawnSync } from "node:child_process";
import path from "node:path";

const action = process.argv[2];
if (action !== "setup" && action !== "start") {
  console.error("usage: node tools/comfyui/run.mjs <setup|start>");
  process.exit(2);
}

const dir = import.meta.dirname;
const isWindows = process.platform === "win32";

const [command, args] = isWindows
  ? [
      "powershell.exe",
      ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path.join(dir, `${action}.ps1`)],
    ]
  : ["bash", [path.join(dir, `${action}.sh`)]];

const result = spawnSync(command, args, { stdio: "inherit" });
if (result.error) {
  console.error(`failed to run ${command}: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
