export default function BhModal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	return (
		<>
			{children}
			{title}
		</>
	);
}
