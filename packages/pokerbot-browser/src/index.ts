// pokerbot-browser: the browser client for the Pokerbot engine (pokerbot-wasm).
//
// * createPokerbotClient: async session client (Web Worker by default)
// * PokerbotTable: reusable React table component
// * Typed DTOs for the engine's JSON contract (docs/WEB_INTEGRATION.md)

export { createPokerbotClient, toSeed } from './client';
export type { ClientOptions, PokerbotClient, StartOptions } from './client';
export { PokerbotTable } from './PokerbotTable';
export type { PokerbotTableProps } from './PokerbotTable';
export { resolveTheme, sepiaTheme, terminalTheme } from './theme';
export type { PokerbotTheme, ThemeProp } from './theme';
export * from './types';
