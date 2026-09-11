import { invokeNative } from "./nativeBridge";

export type CompSharePowerState =
  | "running"
  | "stopped"
  | "starting"
  | "stopping"
  | "unknown";
export type CompShareRunningMode =
  | "gpu"
  | "noGpu"
  | "stopped"
  | "transitioning"
  | "unknown";
export type CompShareStartMode = "gpu" | "noGpu";

export interface CompShareConfiguration {
  credentialsStored: boolean;
  boundInstanceId?: string;
  region?: string;
  zone?: string;
  projectId?: string;
}

export interface CompShareBalance {
  amount?: string;
  amountAvailable?: string;
  amountCredit?: string;
  amountFree?: string;
  amountFreeze?: string;
}

export interface CompShareInstance {
  instanceId: string;
  name?: string;
  region: string;
  zone: string;
  state: CompSharePowerState;
  rawState: string;
  runningMode: CompShareRunningMode;
  cpu?: number;
  memoryMb?: number;
  gpuCount?: number;
  gpuType?: string;
  supportWithoutGpuStart: boolean;
  sshLoginCommand?: string;
  startTime?: number;
  stopTime?: number;
  releaseTime?: number;
  stopSchedulerTime?: number;
  instancePrice?: number;
  projectId?: string;
}

export interface CompShareError {
  code: string;
  message: string;
  retCode?: number;
  requestUuid?: string;
}

function required<T>(value: T | undefined, message: string): T {
  if (value === undefined) throw new Error(message);
  return value;
}

export function normalizeCompShareError(error: unknown): CompShareError {
  if (error && typeof error === "object") {
    const candidate = error as Partial<CompShareError>;
    if (typeof candidate.message === "string") {
      return {
        code: candidate.code ?? "unknown",
        message: candidate.message,
        retCode: candidate.retCode,
        requestUuid: candidate.requestUuid,
      };
    }
  }
  return {
    code: "unknown",
    message: error instanceof Error ? error.message : String(error),
  };
}

export const compShareRepository = {
  configuration: async () =>
    required(
      await invokeNative<CompShareConfiguration>("get_compshare_configuration"),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  saveCredentials: async (publicKey: string, privateKey: string) =>
    required(
      await invokeNative<CompShareConfiguration>("save_compshare_credentials", {
        input: { publicKey, privateKey },
      }),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  clearCredentials: () => invokeNative<void>("clear_compshare_credentials"),
  testConnection: async () =>
    required(
      await invokeNative<{ connected: boolean; balance: CompShareBalance }>(
        "test_compshare_connection",
      ),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  balance: async () =>
    required(
      await invokeNative<CompShareBalance>("get_compshare_balance"),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  listInstances: async (region?: string, zone?: string) =>
    required(
      await invokeNative<CompShareInstance[]>("list_compshare_instances", {
        input: { region, zone },
      }),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  bindInstance: async (instance: CompShareInstance) =>
    required(
      await invokeNative<CompShareInstance>("bind_compshare_instance", {
        input: {
          instanceId: instance.instanceId,
          region: instance.region,
          zone: instance.zone,
          projectId: instance.projectId,
        },
      }),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  boundInstance: async () =>
    required(
      await invokeNative<CompShareInstance>("get_bound_compshare_instance"),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  start: async (mode: CompShareStartMode) =>
    required(
      await invokeNative<{ requestSent: boolean; instance: CompShareInstance }>(
        "start_compshare_instance",
        { mode },
      ),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  stop: async () =>
    required(
      await invokeNative<{ requestSent: boolean; instance: CompShareInstance }>(
        "stop_compshare_instance",
      ),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  setStopDeadline: async (stopTime: number, projectId?: string) =>
    required(
      await invokeNative<{ stopTime: number; instance: CompShareInstance }>(
        "update_compshare_stop_scheduler",
        { input: { stopTime, projectId } },
      ),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
  clearStopDeadline: async () =>
    required(
      await invokeNative<{ deleted: boolean; instance: CompShareInstance }>(
        "delete_compshare_stop_scheduler",
      ),
      "优云智算接口仅可在桌面客户端中使用。",
    ),
};
