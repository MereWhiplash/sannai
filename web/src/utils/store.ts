import { Store } from '@tanstack/store'

interface AppState {
  agentConnected: boolean
  sidebarOpen: boolean
}

export const appStore = new Store<AppState>({
  agentConnected: false,
  sidebarOpen: true,
})

export function setAgentConnected(connected: boolean) {
  appStore.setState((prev) => ({ ...prev, agentConnected: connected }))
}

export function toggleSidebar() {
  appStore.setState((prev) => ({ ...prev, sidebarOpen: !prev.sidebarOpen }))
}
