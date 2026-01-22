import { createContext } from "preact";
import { useContext } from "preact/hooks";

export enum Variants {
	BH,
	YS,
	SR,
	NAP,
}
export const GameContext = createContext<Variants>(Variants.SR);
export const useGame = () => useContext(GameContext);
