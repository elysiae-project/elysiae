type EventCallback<T = unknown> = (data: T) => void;
export class CustomEventManager {
	private handlers: Record<string, EventCallback[]>;

	constructor() {
		this.handlers = {};
	}

	addEventListener<T = unknown>(
		name: string,
		callback: EventCallback<T>,
	): void {
		if (!this.handlers[name]) this.handlers[name] = [];
		this.handlers[name].push(callback as EventCallback);
	}

	removeEventListener<T = unknown>(
		name: string,
		callback: EventCallback<T>,
	): void {
		if (!this.handlers[name]) return;
		this.handlers[name] = this.handlers[name].filter((h) => h !== callback);
	}

	dispatchEvent<T = unknown>(name: string, data: T): void {
		if (!this.handlers[name]) return;
		for (let i = 0; i < this.handlers[name].length; i++) {
			this.handlers[name][i](data);
		}
	}
}
