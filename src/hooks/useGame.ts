import { createContext } from "preact";
import { useContext } from "preact/hooks";
import { Variants } from "../types";

export const GameContext = createContext<Variants>(Variants.SR);
export const useGame = () => useContext(GameContext);
