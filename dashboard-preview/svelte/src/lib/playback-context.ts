import {createContext} from 'svelte';
import type {Status} from './api';

export const [getDashboardContext, setDashboardContext] = createContext<{
  playback: () => Status | null | undefined;
  colorScheme: () => 'dark' | 'light';
}>();
