import { fetch } from "@tauri-apps/plugin-http";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import {
	type LauncherBrandingData,
	type LauncherBrandingRawData,
	type LauncherBrandingRawGameData,
	type LauncherGraphicsData,
	type LauncherGraphicsRawData,
	type LauncherGraphicsRawGameData,
	Variants,
} from "../types";

const LAUNCHER_ID = "VYTpXlbWo8";
const LANGUAGE = "en";
const BH3_EN_ID = "bxPTXSET5t";

interface ApiContextType {
	graphics: LauncherGraphicsData | null;
	branding: LauncherBrandingData | null;
	isLoading: boolean;
	error: string | null;
	refetch: () => void;
}

export const ApiContext = createContext<ApiContextType>({
	graphics: null,
	branding: null,
	isLoading: true,
	error: null,
	refetch: () => {},
});

export const ApiProvider = ({ children }: { children: ComponentChildren }) => {
	const [graphicsData, setGraphicsData] = useState<LauncherGraphicsData | null>(
		null,
	);
	const [brandingData, setBrandingData] = useState<LauncherBrandingData | null>(
		null,
	);
	const [isLoading, setIsLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const abortRef = useRef<AbortController | null>(null);
	const fetchCountRef = useRef(0);

	const fetchData = () => {
		if (abortRef.current) {
			abortRef.current.abort();
		}
		const controller = new AbortController();
		abortRef.current = controller;
		const fetchId = ++fetchCountRef.current;

		setIsLoading(true);
		setError(null);

		const graphicsUrl = `https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getAllGameBasicInfo"].join("/")}?launcher_id=${LAUNCHER_ID}&language=${LANGUAGE}`;
		const brandingUrl = `https://${["sg", "hyp", "api"].join("-")}.hoyoverse.com/${["hyp", "hyp-connect", "api", "getGames"].join("/")}?launcher_id=${LAUNCHER_ID}`;

		const fetchGraphics = fetch(graphicsUrl, { signal: controller.signal })
			.then((res) => res.json())
			.then((data: LauncherGraphicsRawData) => {
				if (data.message !== "OK") throw new Error(data.message);
				const formatted = data.data.game_info_list.reduce(
					(acc: LauncherGraphicsData, game: LauncherGraphicsRawGameData) => {
						let id: Variants | undefined;
						switch (game.game.biz) {
							case "bh3_global":
								if (game.game.id === BH3_EN_ID) id = Variants.BH3;
								break;
							case "hk4e_global":
								id = Variants.HK4E;
								break;
							case "hkrpg_global":
								id = Variants.HKRPG;
								break;
							case "nap_global":
								id = Variants.NAP;
								break;
						}
						if (typeof id === "undefined" || game.backgrounds.length === 0)
							return acc;

						const bg =
							game.backgrounds.find(
								(b) => b.type === "BACKGROUND_TYPE_VIDEO",
							) ?? game.backgrounds[0];

						acc[id] = {
							backgroundImage: bg.background.url,
							backgroundVideo: bg.video.url,
							backgroundVideoOverlay: bg.theme.url,
							icon: bg.icon.url,
						};
						return acc;
					},
					{} as LauncherGraphicsData,
				);
				if (fetchId === fetchCountRef.current) {
					setGraphicsData(formatted);
				}
			});

		const fetchBranding = fetch(brandingUrl, { signal: controller.signal })
			.then((res) => res.json())
			.then((data: LauncherBrandingRawData) => {
				if (data.message !== "OK") throw new Error(data.message);
				const formatted = data.data.games.reduce(
					(acc: LauncherBrandingData, game: LauncherBrandingRawGameData) => {
						let id: Variants | undefined;
						switch (game.biz) {
							case "bh3_global":
								id = Variants.BH3;
								break;
							case "hk4e_global":
								id = Variants.HK4E;
								break;
							case "hkrpg_global":
								id = Variants.HKRPG;
								break;
							case "nap_global":
								id = Variants.NAP;
								break;
						}
						if (typeof id === "undefined") return acc;
						acc[id] = {
							id: game.id,
							icon: game.display.icon.url,
							iconLarge: game.display.shortcut.url,
						};
						return acc;
					},
					{} as LauncherBrandingData,
				);
				if (fetchId === fetchCountRef.current) {
					setBrandingData(formatted);
				}
			});

		Promise.all([fetchGraphics, fetchBranding])
			.then(() => {
				if (fetchId === fetchCountRef.current) {
					setIsLoading(false);
				}
			})
			.catch((err) => {
				if (controller.signal.aborted) return;
				if (fetchId === fetchCountRef.current) {
					setError(err instanceof Error ? err.message : String(err));
					setIsLoading(false);
				}
			});
	};

	useEffect(() => {
		fetchData();
		return () => {
			abortRef.current?.abort();
		};
	}, []);

	return (
		<ApiContext.Provider
			value={{
				graphics: graphicsData,
				branding: brandingData,
				isLoading,
				error,
				refetch: fetchData,
			}}
		>
			{children}
		</ApiContext.Provider>
	);
};
