export const fadeInOut = {
	initial: { opacity: 0 },
	animate: { opacity: 1 },
	exit: { opacity: 0 },
	transition: { duration: 0.25, ease: "easeInOut" },
};

export const springyFadeInOut = {
	initial: {
		opacity: 0,
		scale: 0,
	},
	animate: {
		opacity: 1,
		scale: 1.0,
	},
	exit: {
		opacity: 0,
		scale: 0,
	},
	transition: {
		duration: 0.35,
		ease: "easeInOut",
	},
};
