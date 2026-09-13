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
export type ComputeKeepAlivePolicy = "economy" | "availability" | "continuous";
export type ComputeInstanceRole = "primary" | "elastic" | "user_managed" | "test";
export type ComputeInstanceOwnership = "zhihua_managed" | "user_managed";
export type ComputeCleanupPolicy = "retain" | "release_when_idle";
export type ComputeLifecycleState =
  | "discovered" | "creating" | "starting" | "preparing" | "idle" | "busy"
  | "draining" | "stopping" | "retained" | "terminating" | "terminated"
  | "unknown" | "error";

export interface ComputePolicySnapshot {
  policy: ComputeKeepAlivePolicy;
  idleShutdownMinutes: number | null;
  hardLimitMinutes: number;
}

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
  diskPrice?: number;
  imagePrice?: number;
  imageId?: string;
  chargeType?: string;
  projectId?: string;
}

export interface CompShareCreateSpec {
  region: string;
  zone: string;
  gpuType: string;
  gpuCount: number;
  cpu: number;
  memoryMb: number;
  imageId: string;
  machineType?: string;
  minimalCpuPlatform?: string;
  chargeType?: string;
  bootDiskType?: string;
  bootDiskSizeGb?: number;
  projectId?: string;
}

export interface CompShareCapacitySpec {
  cpu: number;
  memoryGb: number;
  gpuCount: number;
  resourceEnough: boolean;
}

export interface CompShareCreatePreflight {
  spec: CompShareCreateSpec;
  checkedAt: string;
  capacityAvailable: boolean;
  compatibleSpecs: CompShareCapacitySpec[];
  priceDetails: unknown;
  estimatedHourlyPrice?: number;
}

export interface ComputeOperation {
  id: string;
  idempotencyKey: string;
  instanceId?: string;
  action: "create" | "terminate";
  status: "pending" | "succeeded" | "failed" | "unknown";
  requestUuid?: string;
  errorMessage?: string;
  payload: unknown;
  createdAt: string;
  updatedAt: string;
}

export interface ManagedComputeInstance {
  instanceId: string;
  name?: string;
  region: string;
  zone: string;
  projectId?: string;
  role: ComputeInstanceRole;
  ownership: ComputeInstanceOwnership;
  cleanupPolicy: ComputeCleanupPolicy;
  lifecycleState: ComputeLifecycleState;
  platformState: string;
  runningMode: "gpu" | "no_gpu" | "stopped" | "transitioning" | "unknown";
  gpuType?: string;
  gpuCount?: number;
  imageId?: string;
  releaseTime?: number;
  stopTime?: number;
  stopSchedulerTime?: number;
  instancePrice?: number;
  diskPrice?: number;
  currentJobId?: string;
  lastSyncedAt: string;
  missingSince?: string;
  updatedAt: string;
}

export interface ComputeReleaseEligibility {
  allowed: boolean;
  reasons: string[];
}

export type ComputeServiceState =
  | "unknown"
  | "connecting"
  | "waiting_for_gpu"
  | "ready"
  | "incompatible"
  | "unreachable";

export interface ComputeWorkerReadiness {
  instanceId: string;
  state: ComputeServiceState;
  serviceVersion?: string;
  apiVersion?: string;
  workflowManifestVersion?: string;
  modelManifestVersion?: string;
  detail?: string;
  checkedAt: string;
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
  computePolicy: async () =>
    required(
      await invokeNative<ComputePolicySnapshot>("get_compute_policy"),
      "算力策略仅可在桌面客户端中使用。",
    ),
  setComputePolicy: async (policy: ComputeKeepAlivePolicy) =>
    required(
      await invokeNative<ComputePolicySnapshot>("set_compute_policy", { policy }),
      "算力策略仅可在桌面客户端中使用。",
    ),
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
  preflightCreate: async (spec: CompShareCreateSpec) =>
    required(
      await invokeNative<CompShareCreatePreflight>("preflight_compshare_create", { spec }),
      "实例创建预检仅可在桌面客户端中使用。",
    ),
  createManagedInstance: async (input: {
    idempotencyKey: string;
    name: string;
    spec: CompShareCreateSpec;
    role: "elastic" | "test";
    confirmed: boolean;
  }) =>
    required(
      await invokeNative<ComputeOperation>("create_managed_compshare_instance", { input }),
      "实例创建仅可在桌面客户端中使用。",
    ),
  managedInstances: async () =>
    required(
      await invokeNative<ManagedComputeInstance[]>("list_managed_compute_instances"),
      "实例中心仅可在桌面客户端中使用。",
    ),
  workerReadiness: async () =>
    required(
      await invokeNative<ComputeWorkerReadiness[]>("list_compute_worker_readiness"),
      "worker 状态仅可在桌面客户端中使用。",
    ),
  reconcileInstances: async () =>
    required(
      await invokeNative<ManagedComputeInstance[]>("reconcile_compute_instances"),
      "实例中心仅可在桌面客户端中使用。",
    ),
  releaseEligibility: async (instanceId: string) =>
    required(
      await invokeNative<ComputeReleaseEligibility>("get_compute_release_eligibility", { instanceId }),
      "实例中心仅可在桌面客户端中使用。",
    ),
  releaseManagedInstance: async (instanceId: string, releaseDataDisk = false) =>
    required(
      await invokeNative<{ status: "pending" | "succeeded" | "failed" | "unknown"; errorMessage?: string }>(
        "release_managed_compshare_instance",
        {
          input: {
            idempotencyKey: crypto.randomUUID(),
            instanceId,
            releaseDataDisk,
            confirmed: true,
          },
        },
      ),
      "实例释放仅可在桌面客户端中使用。",
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
