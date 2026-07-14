// Web 数据源：走现有门户 REST。服务器由门户/Admin 管理，前端不做本地增删。
import { api, type Me, type Server, type SettingsBody } from "../api";
import type { DataSource, ServerInput } from "./index";

const unsupported = (): never => {
  throw new Error("web 模式下服务器由门户管理，请用管理界面");
};

export const webData: DataSource = {
  loadMe: (): Promise<Me> => api.me(),
  saveCredential: (b: SettingsBody) => api.saveSettings(b),
  upsertServer: (_s: ServerInput): Promise<Server[]> => unsupported(),
  delServer: (_name: string): Promise<Server[]> => unsupported(),
  delCredential: (_name: string): Promise<void> => unsupported(),
  // web 无保险库：恒为已解锁（凭据由门户后端管）。
  vaultState: async () => ({ exists: true, unlocked: true }),
  vaultUnlock: async () => {},
  vaultLock: async () => {},
  getUsername: async () => "",
  setUsername: async () => {},
  readSshConfig: () => unsupported(),
  importHosts: () => unsupported(),
  pickSshConfigFile: () => unsupported(),
  loadTeam: () => unsupported(),
  pickTeamFile: () => unsupported(),
};
