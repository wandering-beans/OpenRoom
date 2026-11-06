CREATE TABLE `download_counts` (
	`platform` text PRIMARY KEY NOT NULL,
	`clicks` integer NOT NULL,
	CONSTRAINT "valid_platform" CHECK("download_counts"."platform" IN ('windows', 'macos')),
	CONSTRAINT "nonnegative_clicks" CHECK("download_counts"."clicks" >= 0)
);
