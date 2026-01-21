import { createContext } from 'preact';
import {useContext} from "preact/hooks";

export type variants = "hi3" | "genshin" | "hsr" | "zzz" | null | undefined;
export const GameContext = createContext<variants>('hsr');
export const useGame = () => useContext(GameContext);