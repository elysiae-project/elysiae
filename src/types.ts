/** biome-ignore-all lint/suspicious/noExplicitAny: Types that use any need them. Currently, those types fetch from an online webpoint that currently returns null but could be updated in the future */
import type React from "react";

// Enums

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

// Primitive / Alias Types

export type GameCodes = "bh3" | "hk4e" | "hkrpg" | "nap";
export type AppModules = "proton" | "jadeite";
export type ComponentSize = "xs" | "sm" | "md" | "lg" | "xl";

// Game

export type GameData = {
	gameCode: string;
	gameDir: string;
	requestedLanguage: string;
};

export type ResumeInfo = {
	gameId: string;
	downloadType: "fresh" | "update" | "preinstall";
};

// Launcher – Graphics

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
		backgroundVideo: string | null;
		backgroundVideoOverlay: string;
		icon: string;
	};
};

// Launcher – Branding
export type LauncherBackgroundData = {
	[key in Variants]: {
		backgroundVideo: string | null;
		backgroundImage: string | null;
	};
};

export type LauncherBackgroundRawData = {
	[key in GameCodes]: LauncherBackgroundAsset[];
};

export type LauncherBackgroundAsset = {
	image: string | null;
	video: string | null;
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

// Proton / Components

export type ProtonComponent = {
	componentName: AppModules;
	extractTo: string;
	saveTo: string;
	postInstall?: () => Promise<void>;
};

export type ProtonComponentData = {
	proton: string | null;
	jadeite: string | null;
};

export type ModuleData = {
	tag: string;
	download_url: string;
	hash: string;
};

// Sophon / Download Progress

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
	| { type: "installingPlugins"; current_plugin: string; total_plugins: number }
	| {
			type: "downloadingPlugin";
			name: string;
			downloaded_bytes: number;
			total_bytes: number;
	  }
	| { type: "finished" };

export type ProtonSetupProgress =
	| {
			type: "protonSetupDownloading";
			component: string;
			downloaded_bytes: number;
			total_bytes: number;
	  }
	| { type: "protonSetupExtracting"; component: string }
	| { type: "protonSetupInstalling"; component: string }
	| { type: "protonSetupFinished" };

// Modal

export type ModalHandle = {
	open: () => void;
	close: () => void;
	toggle: (state: boolean) => void;
};

export type ModalProps = {
	title?: string;
	width?: number;
	height?: number;
	closeable?: boolean;
	children: React.ReactNode;
};

// Settings / Options

export type Settings = {
	version: number;
	isFirstLaunch: boolean;
	lastUsedVersion: string;
	selectedGame: GameCodes;
	voLanguage: string;
	blockNotifications: boolean;
	createShortcuts: boolean;
	autoUpdate: boolean;
	autoPreload: boolean;
	installedComponents: InstalledComponentsData;
	cachedBackgrounds: SettingsCachedBackgrounds;
};

export type InstalledComponentsData = {
	proton: string | null;
	jadeite: string | null;
};

// New format for cached backgrounds. will remove older CachedBackground type later
export type SettingsCachedBackgrounds = {
	[key in Variants]: {
		type: "video" | "image";
		path: string;
	}[];
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

export type CachedBackgrounds = {
	[key in Variants]: string[];
};
