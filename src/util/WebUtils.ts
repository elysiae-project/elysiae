/**
 * @param url The URL to check
 * @returns HTTP status code of the URL
 */
export const getHttpStatus = async (url: string): Promise<number> => {
	console.log(`Checking HTTP Status of ${url}`);
	return new Promise((resolve, reject) => {
		if (!isURLValid(url)) {
			reject(`getHttpStatus: URL ${url} is invalid`);
		}
		fetch(url, {
			method: "GET",
		}).then((response) => {
			resolve(response.status);
		});
	});
};

/**
 * @param verifyingString the string you want to verify
 * @returns boolean value based on weather verifyingString is a valid http URL or not
 */
export const isURLValid = (verifyingString: string): boolean => {
	try {
		const testURL = new URL(verifyingString);
		return testURL.protocol === "http:" || testURL.protocol === "https:";
	} catch {
		return false;
	}
};

/**
 * @param url link to an API
 * @returns JavaScipt Object from API URL
 */
export const getApiJson = async (url: string): Promise<any> => {
	return new Promise((resolve, reject) => {
		if (!isURLValid(url)) {
			reject(`getApiJson: URL ${url} is invalid`);
		}
		fetch(url, {
			method: "GET",
		}).then((response) => {
			if (response.status === 200) {
				response
					.json()
					.then((json) => {
						resolve(json);
					})
					.catch((e) => {
						reject(`getApiJson: ${e}`);
					});
			} else {
				reject(`getAPIJson: ${url} returned status code ${response.status}`);
			}
		});
	});
};

/**
 * @param url link to a github api link
 * @returns Object containing only useful information used by yoohoo when getting data from GitHub repositories
 */
export const getGithubInfo = async (url: string): Promise<any> => {
	return new Promise((resolve, reject) => {
		if (!url.includes("api.github.com")) {
			reject("URL Does not point to the GitHub API");
		}
		getApiJson(url)
			.then((json) => {
				resolve({
					downloadURL: json.assets[0].browser_download_url,
					hash: json.assets[0].digest.slice(7),
					tagName: json.tag_name,
				});
			})
			.catch((e) => {
				reject(e);
			});
	});
};
