import { createContext } from "preact";
import { useContext } from "preact/hooks";
import { Variants } from "../types";

export const GameContext = createContext<Variants>(Variants.YS);
export const useGame = () => useContext(GameContext);
