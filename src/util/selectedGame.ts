import { createContext } from 'preact';
import {useContext} from "preact/hooks";

export type variants = "bh" | "ys" | "sr" | "nap" | null | undefined;
export const GameContext = createContext<variants>('sr');
export const useGame = () => useContext(GameContext);
