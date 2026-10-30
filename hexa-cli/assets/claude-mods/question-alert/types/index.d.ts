export type Pending = string | null

declare module 'claude-code' {
  interface PluginState {
    'question-alert': { pending: Pending }
  }
}
