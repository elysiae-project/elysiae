export enum Variants {
	BH,
	YS,
	SR,
	NAP,
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

export type LauncherPkgRawData = {
	retcode: number;
	message: string;
	data: {
		game_packages: LauncherGamePkgRawData[];
	};
};

export type LauncherGamePkgRawData = {
	game: {
		id: string;
		biz: string;
	};
	main: {
		major: {
			version: string;
			game_pkgs: GamePkg[];
		};
	};
	patches: {
		version: string;
		game_pkgs: GamePkg[];
		audio_pkgs: AudioPkg[];
		res_list_url: string;
	}[];
	required_client_version: string;
	pre_download: {
		// TODO: Refine these properties when a preinstal is available (look at the values when they are filled in)
		major: null | string;
		patches: [];
		required_client_version: string;
	};
};
export type GamePkg = {
	url: string;
	md5: string;
	size: string;
	decompressed_size: string;
};
export type AudioPkg = {
	language: string;
	url: string;
	md5: string;
	size: string;
	decompressed_size: string;
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
		icon: string;
		iconLarge: string;
	};
};

export type LauncherPkgData = {
	[key in Variants]: {
		main: {
			major: {
				version: string;
				game_pkgs: GamePkg[];
			};
		};
		patches: {
			version: string;
			game_pkgs: GamePkg[];
			audio_pkgs: AudioPkg[];
			res_list_url: string;
		}[];
		required_client_version: string;
		pre_download: {
			// TODO: Refine these properties when a preinstal is available (look at the values that are filled in)
			major: null | string;
			patches: [];
			required_client_version: string;
		};
	};
};

export type Download = { 
	downloaded: number;
	total: number;
}
