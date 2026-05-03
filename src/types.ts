import React from "react-dom/src";

export enum Variants {
	BH3,
	HK4E,
	HKRPG,
	NAP,
}

export enum BhServers {
	GLB,
	JP,
	KR,
	SEA,
	TW,
}

export type LauncherGraphicsRawGameData = {
	game: {
		id: string;
		biz: string;
	};
	backgrounds: {
		id: string;
		background: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		icon: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		video: {
			url: string;
			size: number;
		};
		theme: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		type: string;
	}[];
};

export type LauncherGraphicsRawData = {
	retcode: number;
	message: string;
	data: {
		game_info_list: LauncherGraphicsRawGameData[];
	};
};
export type LauncherGraphicsData = {
	[key in Variants]: {
		backgroundImage: string;
		backgroundVideo: string;
		backgroundVideoOverlay: string;
		icon: string;
	};
};

export type LauncherBrandingRawGameData = {
	id: string;
	biz: string;
	display: {
		language: string;
		name: string;
		icon: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		title: string;
		subtitle: string;
		background: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		logo: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		thumbnail: {
			url: string;
			link: string;
			login_state_in_link: boolean;
		};
		korea_rating: any;
		shortcut: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
		wpf_icon?: {
			url: string;
			hover_url: string;
			link: string;
			login_state_in_link: boolean;
			md5: string;
			size: number;
		};
	};
	reservation: any;
	display_status: string;
	game_server_configs: {
		i18n_name: string;
		i18n_description: string;
		package_name: string;
		auto_scan_registry_key: string;
		package_detection_info: string;
		game_id: string;
		reservation: any;
		display_status: string;
	}[];
};

export type LauncherBrandingRawData = {
	retcode: number;
	message: string;
	data: {
		games: LauncherBrandingRawGameData[];
	};
};
export type LauncherBrandingData = {
	[key in Variants]: {
		id: string;
		icon: string;
		iconLarge: string;
	};
};

export type WineComponent = {
	componentName: string;
	extractTo: string;
	saveTo: string;
	postInstall?: () => Promise<void>;
};

export type ComponentData = {
	tag: string;
	download_url: string;
	hash: string;
};

export type WineModule = {
	name: string;
	downloadLink: string;
	moduleType: "exe" | "dll32" | "dll64";
};

export type SophonProgress =
	| { type: "fetchingManifest" }
	| { type: "calculatingDownloads"; checked_files: number; total_files: number }
	| {
			type: "downloading";
			downloaded_bytes: number;
			total_bytes: number;
			speed_bps: number;
			eta_seconds: number;
	  }
	| { type: "paused"; downloaded_bytes: number; total_bytes: number }
	| { type: "assembling"; assembled_files: number; total_files: number }
	| {
			type: "verifying";
			scanned_files: number;
			total_files: number;
			error_count: number;
	  }
	| { type: "warning"; message: string }
	| { type: "error"; message: string }
	| { type: "finished" };

export type GameData = {
	gameCode: string;
	gameDir: string;
	requestedLanguage: string;
};

export type GameCodes = "bh3" | "hk4e" | "hkrpg" | "nap";
export type AppOptions =
	| "selectedGame"
	| "voLanguage"
	| "blockNotifications"
	| "syncMethod";

export type ResumeInfo = {
	gameId: string;
	downloadType: "fresh" | "update" | "preinstall";
};

export type ModalHandle = {
	open: () => void;
	close: () => void;
	toggle: (state: boolean) => void;
};

export type ModalProps = {
	title: string;
	width?: number;
	height?: number;
	children: React.ReactNode;
};

type BaseOption<T, V> = {
	name: string;
	type: T;
	getValue: () => Promise<V>;
	setValue: (value: V) => Promise<void>;
};

type DropdownOption = BaseOption<"dropdown", string> & {
	labels: string[];
	values: string[];
};

type BooleanOption = BaseOption<"boolean", boolean>;

export type Option = DropdownOption | BooleanOption;
