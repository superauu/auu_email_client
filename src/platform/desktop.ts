import { isTauri, invoke } from '@tauri-apps/api/core';
import type { SkinCatalog } from '../themes/contract';

export interface AppInfo {
  version: string;
  dataDirectory: string;
  skinDirectory: string;
  databaseReady: boolean;
}
export const isDesktop = () => isTauri();
export const desktop = {
  async info(): Promise<AppInfo | null> { return isDesktop() ? invoke('get_app_info') : null; },
  async listSkins(): Promise<SkinCatalog> {
    return isDesktop() ? invoke('list_user_skins') : { skins: [], warnings: [] };
  },
  async openSkinDirectory(): Promise<void> {
    if (!isDesktop()) throw new Error('请在桌面版中打开皮肤目录。');
    await invoke('open_skin_directory');
  },
  async windowAction(action: 'minimize' | 'maximize' | 'close' | 'drag'): Promise<void> {
    if (!isDesktop()) return;
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const window = getCurrentWindow();
    if (action === 'minimize') await window.minimize();
    if (action === 'maximize') await window.toggleMaximize();
    if (action === 'close') await window.close();
    if (action === 'drag') await window.startDragging();
  },
};
