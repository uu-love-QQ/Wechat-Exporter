use anyhow::Result;
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};

const KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const ZSTD_HELLO: &[u8] = &[
    0x28, 0xB5, 0x2F, 0xFD, 0x04, 0x58, 0x81, 0x00, 0x00, 0x63, 0x6F, 0x6D, 0x70, 0x72, 0x65, 0x73,
    0x73, 0x65, 0x64, 0x20, 0x68, 0x65, 0x6C, 0x6C, 0x6F, 0xEF, 0xC0, 0x7C, 0xA3,
];

fn md5_hex(value: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(path);
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!("PRAGMA key = \"x'{KEY_HEX}'\";"))?;
    Ok(conn)
}

fn create_message_table(conn: &Connection, table: &str) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE {table} (
            local_id INTEGER PRIMARY KEY,
            server_id INTEGER,
            local_type INTEGER,
            sort_seq INTEGER,
            real_sender_id INTEGER,
            create_time INTEGER,
            status INTEGER,
            upload_status INTEGER,
            download_status INTEGER,
            server_seq INTEGER,
            origin_source INTEGER,
            source TEXT,
            message_content BLOB,
            compress_content BLOB,
            packed_info_data BLOB,
            WCDB_CT_message_content INTEGER,
            WCDB_CT_source INTEGER
        );"
    ))?;
    Ok(())
}

fn insert_message(
    conn: &Connection,
    table: &str,
    local_id: i64,
    server_id: i64,
    local_type: i64,
    sort_seq: i64,
    sender: i64,
    create_time: i64,
    content: &[u8],
    ct: i64,
    source: Option<&str>,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table}(local_id,server_id,local_type,sort_seq,real_sender_id,create_time,status,upload_status,download_status,server_seq,origin_source,source,message_content,compress_content,packed_info_data,WCDB_CT_message_content,WCDB_CT_source) VALUES (?1,?2,?3,?4,?5,?6,0,0,0,0,0,?7,?8,NULL,NULL,?9,0)"
        ),
        params![
            local_id,
            server_id,
            local_type,
            sort_seq,
            sender,
            create_time,
            source.unwrap_or(""),
            content,
            ct
        ],
    )?;
    Ok(())
}

fn build(out: &Path) -> Result<()> {
    let account = out.join("xwechat_files").join("wxid_testuser_abcd");
    let db_storage = account.join("db_storage");
    let alice = "wxid_alice";
    let group = "123456789@chatroom";

    // Contacts
    {
        let conn = open(&db_storage.join("contact").join("contact.db"))?;
        conn.execute_batch(
            "CREATE TABLE contact (
                id INTEGER PRIMARY KEY,
                username TEXT,
                local_type INTEGER,
                alias TEXT,
                encrypt_username TEXT,
                flag INTEGER,
                delete_flag INTEGER,
                verify_flag INTEGER,
                remark TEXT,
                remark_quan_pin TEXT,
                remark_pin_yin_initial TEXT,
                nick_name TEXT,
                pin_yin_initial TEXT,
                quan_pin TEXT,
                big_head_url TEXT,
                small_head_url TEXT,
                head_img_md5 TEXT,
                chat_room_notify TEXT,
                is_in_chat_room INTEGER,
                description TEXT,
                extra_buffer BLOB,
                chat_room_type INTEGER
            );
            CREATE TABLE chatroom_member (room_id INTEGER, member_id INTEGER);
            CREATE TABLE name2id (user_name TEXT);",
        )?;
        let people = [
            (1, "wxid_testuser", "Me", "本人"),
            (2, "wxid_alice", "Alice", "Alice Remark"),
            (3, "wxid_bob", "Bob", ""),
            (4, group, "Test Group", "测试群"),
        ];
        for (id, username, nick, remark) in people {
            conn.execute(
                "INSERT INTO contact(id,username,local_type,alias,remark,nick_name,delete_flag) VALUES (?1,?2,?3,?4,?5,?6,0)",
                params![id, username, if username.ends_with("@chatroom") { 2 } else { 1 }, if username == "wxid_alice" { "alice_alias" } else { "" }, remark, nick],
            )?;
        }
        for (row, user) in [(1, "wxid_testuser"), (2, "wxid_alice"), (3, "wxid_bob"), (4, group)] {
            conn.execute("INSERT INTO name2id(rowid,user_name) VALUES (?1,?2)", params![row, user])?;
        }
        for (room, member) in [(4, 1), (4, 2), (4, 3)] {
            conn.execute(
                "INSERT INTO chatroom_member(room_id,member_id) VALUES (?1,?2)",
                params![room, member],
            )?;
        }
    }

    // Sessions
    {
        let conn = open(&db_storage.join("session").join("session.db"))?;
        conn.execute_batch(
            "CREATE TABLE SessionTable (
                username TEXT PRIMARY KEY,
                type INTEGER,
                unread_count INTEGER,
                unread_first_msg_srv_id INTEGER,
                unread_first_pat_msg_local_id INTEGER,
                unread_first_pat_msg_sort_seq INTEGER,
                is_hidden INTEGER,
                summary TEXT,
                draft TEXT,
                status INTEGER,
                last_timestamp INTEGER,
                sort_timestamp INTEGER,
                last_clear_unread_timestamp INTEGER,
                last_msg_locald_id INTEGER,
                last_msg_type INTEGER,
                last_msg_sub_type INTEGER,
                last_msg_sender TEXT,
                last_sender_display_name TEXT,
                last_msg_ext_type INTEGER
            );",
        )?;
        conn.execute(
            "INSERT INTO SessionTable(username,type,unread_count,sort_timestamp,last_timestamp,last_msg_type,last_msg_sub_type,last_msg_sender,last_sender_display_name,is_hidden) VALUES (?1,2,1,1703001700,1703001700,1,0,?2,'Alice',0)",
            params![group, alice],
        )?;
        conn.execute(
            "INSERT INTO SessionTable(username,type,unread_count,sort_timestamp,last_timestamp,last_msg_type,last_msg_sub_type,last_msg_sender,last_sender_display_name,is_hidden) VALUES (?1,1,0,1703001800,1703001800,49,2000,?2,'Alice',0)",
            params![alice, alice],
        )?;
    }

    // Message shard 0
    {
        let conn = open(&db_storage.join("message").join("message_0.db"))?;
        conn.execute_batch(
            "CREATE TABLE Name2Id (user_name TEXT, is_group INTEGER);
             INSERT INTO Name2Id(rowid,user_name,is_group) VALUES (1,'wxid_testuser',0),(2,'wxid_alice',0),(3,'wxid_bob',0),(4,'123456789@chatroom',1);",
        )?;
        let group_table = format!("Msg_{}", md5_hex(group));
        let alice_table = format!("Msg_{}", md5_hex(alice));
        create_message_table(&conn, &group_table)?;
        create_message_table(&conn, &alice_table)?;
        // Voice payloads live in a media table, keyed by chat rowid and time.
        conn.execute_batch(
            "CREATE TABLE VoiceInfo (
                chat_name_id INTEGER,
                create_time INTEGER,
                local_id INTEGER,
                svr_id INTEGER,
                voice_data BLOB,
                data_index INTEGER
            );",
        )?;
        let pcm: Vec<u8> = (0..12000i32)
            .map(|i| (((i % 200) - 100) * 120) as i16)
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let silk = silk_codec::encode_silk(pcm.as_slice(), 24000, 24000, true)
            .map_err(|error| anyhow::anyhow!("silk encode: {error:?}"))?;
        conn.execute(
            "INSERT INTO VoiceInfo VALUES (4, ?1, 8, 9008, ?2, 0)",
            rusqlite::params![1703001670000i64, silk],
        )?;

        insert_message(
            &conn,
            &group_table,
            1,
            9001,
            1,
            1,
            2,
            1703001600,
            b"hello group",
            0,
            None,
        )?;
        insert_message(
            &conn,
            &group_table,
            2,
            9002,
            1,
            2,
            1,
            1703001610,
            b"hi from me",
            0,
            None,
        )?;
        let image_xml = format!(
            "<msg><img md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" /></msg>"
        );
        insert_message(
            &conn,
            &group_table,
            3,
            9003,
            3,
            3,
            3,
            1703001620,
            image_xml.as_bytes(),
            0,
            None,
        )?;
        let link_xml = "<msg><appmsg><title>Example Link</title><type>5</type><url>https://example.com</url></appmsg></msg>";
        insert_message(
            &conn,
            &group_table,
            4,
            9004,
            49,
            4,
            2,
            1703001630,
            link_xml.as_bytes(),
            0,
            None,
        )?;
        let quote_xml = "<msg><appmsg><title>reply body</title><type>57</type><refermsg><type>1</type><svrid>9003</svrid><displayname>Alice</displayname><content>original text</content><createtime>1703001620</createtime></refermsg></appmsg></msg>";
        insert_message(
            &conn,
            &group_table,
            5,
            9005,
            244813135921,
            5,
            3,
            1703001640,
            quote_xml.as_bytes(),
            0,
            None,
        )?;
        insert_message(
            &conn,
            &group_table,
            6,
            9006,
            10000,
            6,
            1,
            1703001650,
            b"Alice joined the group",
            0,
            None,
        )?;
        insert_message(
            &conn,
            &group_table,
            7,
            9007,
            1,
            7,
            2,
            1703001660,
            ZSTD_HELLO,
            4,
            None,
        )?;
        let record_xml = "<msg><appmsg><title>草帽Wantu与Harry的聊天记录</title><des>Harry: 第一条</des><type>19</type><recorditem><![CDATA[<recordinfo><title>草帽Wantu与Harry的聊天记录</title><desc>Harry: 第一条</desc><datalist count=\"2\"><dataitem datatype=\"1\"><datadesc>第一条</datadesc><sourcename>Harry</sourcename><sourcetime>2026-11-03 07:51:59</sourcetime></dataitem><dataitem datatype=\"1\"><datadesc>第二条</datadesc><sourcename>草帽Wantu</sourcename><sourcetime>2026-11-03 07:52:10</sourcetime></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>";
        insert_message(
            &conn,
            &group_table,
            11,
            9011,
            49,
            11,
            2,
            1703001700,
            record_xml.as_bytes(),
            0,
            None,
        )?;
        let file_xml = "<msg><appmsg><type>6</type><title>report.html</title><appattach><totallen>30428</totallen><cdnattachurl>3057aa</cdnattachurl><aeskey>9f079e18611d58504594d8084036dd55</aeskey><fileext>html</fileext></appattach></appmsg></msg>";
        insert_message(
            &conn,
            &group_table,
            9,
            9009,
            49,
            9,
            3,
            1703001680,
            file_xml.as_bytes(),
            0,
            None,
        )?;
        let missing_quote_xml = "<msg><appmsg><title>reply to gone</title><type>57</type><refermsg><type>1</type><svrid>123456789012345678</svrid><displayname>Ghost</displayname><content>vanished text</content><createtime>1703000000</createtime></refermsg></appmsg></msg>";
        insert_message(
            &conn,
            &group_table,
            10,
            9010,
            49,
            10,
            2,
            1703001690,
            missing_quote_xml.as_bytes(),
            0,
            None,
        )?;
        let voice_xml = "<msg><voicemsg endflag=\"1\" voiceformat=\"4\" voicelength=\"500\" length=\"400\"/></msg>";
        insert_message(
            &conn,
            &group_table,
            8,
            9008,
            34,
            8,
            1,
            1703001670,
            voice_xml.as_bytes(),
            0,
            None,
        )?;

        insert_message(
            &conn,
            &alice_table,
            1,
            8001,
            1,
            1,
            2,
            1703001700,
            b"direct hello",
            0,
            None,
        )?;
        let transfer_xml = "<msg><appmsg><title>transfer</title><type>2000</type><feedesc>¥12.00</feedesc><pay_memo>lunch</pay_memo></appmsg></msg>";
        insert_message(
            &conn,
            &alice_table,
            2,
            8002,
            49,
            2,
            2,
            1703001800,
            transfer_xml.as_bytes(),
            0,
            None,
        )?;
    }

    // Historical shard
    {
        let conn = open(&db_storage.join("message").join("message_1.db"))?;
        conn.execute_batch(
            "CREATE TABLE Name2Id (user_name TEXT, is_group INTEGER);
             INSERT INTO Name2Id(rowid,user_name,is_group) VALUES (1,'wxid_testuser',0),(2,'wxid_alice',0);",
        )?;
        let alice_table = format!("Msg_{}", md5_hex(alice));
        create_message_table(&conn, &alice_table)?;
        insert_message(
            &conn,
            &alice_table,
            1,
            7001,
            1,
            1,
            1,
            1690000000,
            b"old message from me",
            0,
            None,
        )?;
    }

    // Plain image file used by --include-media.
    let media_dir = account
        .join("msg")
        .join("attach")
        .join(md5_hex(group))
        .join("2023-12")
        .join("Img");
    fs::create_dir_all(&media_dir)?;
    fs::write(
        media_dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.dat"),
        [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
            0x52,
        ],
    )?;

    println!("{}", out.display());
    Ok(())
}

fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("wechat-fixture"));
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    fs::create_dir_all(&out)?;
    build(&out)?;
    println!("key={KEY_HEX}");
    println!("source={}", out.join("xwechat_files").join("wxid_testuser_abcd").join("db_storage").display());
    Ok(())
}
