import {sql} from 'drizzle-orm';
import {sqliteTable, text, integer, check} from 'drizzle-orm/sqlite-core';

export const downloadCounts = sqliteTable('download_counts', {
  platform: text('platform').primaryKey().notNull(),
  clicks: integer('clicks').notNull()
}, table => [
  check('valid_platform', sql`${table.platform} IN ('windows', 'macos')`),
  check('nonnegative_clicks', sql`${table.clicks} >= 0`)
]);
