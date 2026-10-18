// mako-browser: the browser client for the Mako engine (mako-wasm).
//
// * createMakoClient: async session client (Web Worker by default)
// * MakoTable: reusable React table component
// * Typed DTOs for the engine's JSON contract (docs/WEB_INTEGRATION.md)

export { createMakoClient, toSeed } from './client';
export type { ClientOptions, MakoClient, StartOptions } from './client';
export { MakoTable } from './MakoTable';
export type { MakoTableProps } from './MakoTable';
export { resolveTheme, sepiaTheme, terminalTheme } from './theme';
export type { MakoTheme, ThemeProp } from './theme';
export * from './types';
