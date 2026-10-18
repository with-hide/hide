-- OpenCode 2.0.29's own tables, as `sqlite3 opencode.db .schema` prints them
-- (`project` cut to the column a session's key needs; `session_pending` and
-- `session_inbox` hold prompts not yet admitted as messages and stay empty).
CREATE TABLE `project` (`id` text PRIMARY KEY);
INSERT INTO project VALUES ('prj_1');
CREATE TABLE `session_v2` (
          `id` text PRIMARY KEY,
          `project_id` text NOT NULL,
          `workspace_id` text,
          `parent_id` text,
          `fork_session_id` text,
          `fork_boundary` text,
          `slug` text NOT NULL,
          `directory` text NOT NULL,
          `path` text,
          `title` text,
          `version` text NOT NULL,
          `share_url` text,
          `summary_additions` integer,
          `summary_deletions` integer,
          `summary_files` integer,
          `summary_diffs` text,
          `metadata` text,
          `cost` real DEFAULT 0 NOT NULL,
          `tokens_input` integer DEFAULT 0 NOT NULL,
          `tokens_output` integer DEFAULT 0 NOT NULL,
          `tokens_reasoning` integer DEFAULT 0 NOT NULL,
          `tokens_cache_read` integer DEFAULT 0 NOT NULL,
          `tokens_cache_write` integer DEFAULT 0 NOT NULL,
          `revert` text,
          `permission` text,
          `agent` text,
          `model` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `time_idle` integer,
          `time_viewed` integer,
          `idle_outcome` text,
          `time_compacting` integer,
          `time_archived` integer,
          `time_suspended` integer,
          `resume_attempts` integer DEFAULT 0 NOT NULL,
          CONSTRAINT `fk_session_v2_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
CREATE INDEX `session_v2_project_idx` ON `session_v2` (`project_id`);
CREATE INDEX `session_v2_workspace_idx` ON `session_v2` (`workspace_id`);
CREATE INDEX `session_v2_parent_idx` ON `session_v2` (`parent_id`);
CREATE INDEX `session_v2_time_suspended_idx` ON `session_v2` (`time_suspended`) WHERE "session_v2"."time_suspended" is not null;
CREATE TABLE `session_message` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_session_message_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE UNIQUE INDEX `session_message_session_seq_idx` ON `session_message` (`session_id`,`seq`);
CREATE INDEX `session_message_session_type_seq_idx` ON `session_message` (`session_id`,`type`,`seq`);
CREATE INDEX `session_message_session_time_created_id_idx` ON `session_message` (`session_id`,`time_created`,`id`);
CREATE INDEX `session_message_time_created_idx` ON `session_message` (`time_created`);
CREATE TABLE `session_pending` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `data` text NOT NULL,
          `delivery` text,
          `admitted_seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          CONSTRAINT `fk_session_pending_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE INDEX `session_pending_session_delivery_seq_idx` ON `session_pending` (`session_id`,`delivery`,`admitted_seq`);
CREATE UNIQUE INDEX `session_pending_session_compaction_idx` ON `session_pending` (`session_id`) WHERE "session_pending"."type" = 'compaction';
CREATE UNIQUE INDEX `session_pending_session_admitted_seq_idx` ON `session_pending` (`session_id`,`admitted_seq`);
CREATE TABLE `session_inbox` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `payload` text NOT NULL,
          `delivery` text NOT NULL,
          `enqueued_seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          CONSTRAINT `fk_session_inbox_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE INDEX `session_inbox_session_delivery_seq_idx` ON `session_inbox` (`session_id`,`delivery`,`enqueued_seq`);
CREATE UNIQUE INDEX `session_inbox_session_enqueued_seq_idx` ON `session_inbox` (`session_id`,`enqueued_seq`);

INSERT INTO session_v2 (id, project_id, slug, directory, title, version, time_created, time_updated) VALUES ('ses_0a1b2c3d4e5f60718293a4b5c6', 'prj_1', 'brave-otter', '/work/app', '요청 보기 만들기', '2.0.29', 1790989200000, 1790989331001);
INSERT INTO session_message VALUES ('msg_01', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'user', 1, 1790989200000, 1790989200000, '{"time":{"created":1790989200000},"text":"요청 보기를 만들어줘\n긴 요청의 둘째 줄","files":[],"agents":[]}');
INSERT INTO session_message VALUES ('msg_02', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'assistant', 2, 1790989201000, 1790989231000, '{"time":{"created":1790989201000,"streamed":1790989230500,"completed":1790989231000},"agent":"build","model":{"id":"m","providerID":"p"},"content":[{"type":"reasoning","text":"Ship it.","time":{"created":1790989202000,"completed":1790989203000}},{"type":"tool","id":"c1","name":"shell","executed":false,"state":{"status":"completed","input":{"command":"bash scripts/ship.sh"},"content":[{"type":"text","text":"https://github.com/acme/app/pull/12\n"}],"metadata":{"status":"completed","truncated":false,"exit":0}},"time":{"created":1790989220000,"ran":1790989220100,"completed":1790989230000}}],"finish":"tool-calls","rawFinish":"tool_calls","cost":0.5,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}}');
INSERT INTO session_message VALUES ('msg_03', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'synthetic', 3, 1790989240000, 1790989240000, '{"time":{"created":1790989240000},"text":"Summarize the conversation so far","description":"Summary request"}');
INSERT INTO session_message VALUES ('msg_04', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'user', 4, 1790989260000, 1790989260000, '{"time":{"created":1790989260000},"text":"\nHide letter letter-1 from ci-lead (claude) [request]\nCI 다시 봐줘\nFull letter: hide request show letter-1\n","files":[],"agents":[]}');
INSERT INTO session_message VALUES ('msg_05', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'user', 5, 1790989320000, 1790989320000, '{"time":{"created":1790989320000},"text":"","files":[{"data":"iVBORw0KGgo=","mime":"image/png","source":{"type":"inline"},"name":"clipboard.png"}],"agents":[]}');
INSERT INTO session_message VALUES ('msg_06', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'assistant', 6, 1790989330000, 1790989331000, '{"time":{"created":1790989330000,"streamed":1790989330900,"completed":1790989331000},"agent":"build","model":{"id":"m","providerID":"p"},"content":[{"type":"text","text":"PR을 열었습니다: https://github.com/acme/app/pull/12\n예전 것은 https://github.com/acme/app/pull/99 입니다"}],"finish":"stop","rawFinish":"stop","cost":0.25,"tokens":{"input":20,"output":8,"reasoning":2,"cache":{"read":100,"write":4}}}');
INSERT INTO session_message VALUES ('msg_07', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'idle', 7, 1790989331001, 1790989331001, '{"time":{"created":1790989331001},"outcome":"succeeded"}');
-- A subagent's child session: its records never enter the root conversation.
INSERT INTO session_v2 (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated) VALUES ('ses_0a1b2c3d4e5f60718293child', 'prj_1', 'ses_0a1b2c3d4e5f60718293a4b5c6', 'quiet-heron', '/work/app', 'Explore the code', '2.0.29', 1790989202000, 1790989215000);
INSERT INTO session_message VALUES ('msg_c1', 'ses_0a1b2c3d4e5f60718293child', 'user', 1, 1790989202000, 1790989202000, '{"time":{"created":1790989202000},"text":"subagent task text","files":[],"agents":[]}');
INSERT INTO session_message VALUES ('msg_c2', 'ses_0a1b2c3d4e5f60718293child', 'assistant', 2, 1790989210000, 1790989215000, '{"time":{"created":1790989210000,"streamed":1790989214900,"completed":1790989215000},"agent":"explore","model":{"id":"m","providerID":"p"},"content":[{"type":"text","text":"subagent private answer https://github.com/acme/app/pull/77"}],"finish":"stop","rawFinish":"stop","cost":0,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}}}');
