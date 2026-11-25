import { invoke } from "./invoke";

/** ComfyUI のアプリ内インストール(docs/23、docs/04-ipc-api.md「ComfyUI のインストール」)。型は Rust 側が正。 */

export type InstallStatus = "idle" | "running" | "done" | "failed" | "cancelled";

export interface InstallState {
  status: InstallStatus;
  /** 現在の手順(1 始まり)。実行中以外は 0。 */
  step: number;
  totalSteps: number;
  stepId: string | null;
  received: number | null;
  total: number | null;
  error: { code: string; message: string } | null;
}

export interface InstallRecord {
  manifestVersion: string;
  comfyuiTag: string;
  components: string[];
  steps: Record<string, string>;
  acceptedAt: number;
  complete: boolean;
}

export interface InstallComponent {
  id: string;
  name: string;
  license: string;
  downloadMb: number;
  diskMb: number;
}

export interface InstallBase {
  pythonVersion: string;
  comfyuiTag: string;
  comfyuiLicense: string;
  torchVersion: string;
  torchLicense: string;
  downloadMb: number;
  diskMb: number;
}

export type InstallPlatform = "windows-x64" | "macos-arm64";

export interface InstallView {
  state: InstallState;
  /** 検証まで完了した導入の記録。 */
  record: InstallRecord | null;
  updateAvailable: boolean;
  manifestVersion: string;
  installDir: string;
  platform: InstallPlatform | null;
  base: InstallBase;
  components: InstallComponent[];
  /** 接続する取得元のホスト。 */
  hosts: string[];
}

export type GpuStatus = "found" | "missing" | "notApplicable";

export interface Preflight {
  platform: InstallPlatform | null;
  supported: boolean;
  unsupportedReason: string | null;
  freeBytes: number | null;
  requiredBytes: number;
  enoughSpace: boolean;
  gpu: GpuStatus;
  gpuName: string | null;
  installDir: string;
}

export const engineInstallStatus = () => invoke<InstallView>("engine_install_status");

export const engineInstallPreflight = (components: string[]) =>
  invoke<Preflight>("engine_install_preflight", { components });

/** 実行中のジョブ・起動中の ComfyUI があると `ENGINE_BUSY`、容量不足は `ENGINE_INSTALL_DISK_FULL`。 */
export const engineInstallStart = (components: string[], accepted: boolean) =>
  invoke<InstallState>("engine_install_start", { request: { components, accepted } });

export const engineInstallCancel = () => invoke<void>("engine_install_cancel");

export const engineInstallRemove = (keepData: boolean) =>
  invoke<InstallView>("engine_install_remove", { keepData });

export const engineInstallLogs = (lines?: number) =>
  invoke<{ lines: string[] }>("engine_install_logs", { lines });
