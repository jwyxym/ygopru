pub mod string {
    #![allow(dead_code)]

    use std::ops::Deref;
    use std::sync::OnceLock;

    use binrw::BinRead;
    use binrw::BinWrite;
    use binrw::binrw;
    use binrw::helpers::until_eof;

    /// transform \[u16\] to string. \
    /// return [`None`] if it's illegal.
    pub fn cast_to_string(array: &[u16]) -> Option<String> {
        let mut str = array;
        if let Some(index) = array.iter().position(|&i| i == 0) {
            str = &str[0..index as usize];
        }
        else { return None }
        let body = unsafe { std::slice::from_raw_parts(str.as_ptr() as *const u8, str.len() * 2) };
        let (cow, _, had_errors) = encoding_rs::UTF_16LE.decode(&body);
        if had_errors { None }
        else { Some(cow.to_string()) }
    }

    /// Transform string to \[u16\] without length limit but a \0 in the end.
    pub fn cast_to_c_array(message: &str) -> Vec<u16> {
        let mut vector: Vec<u16> = message.encode_utf16().collect();
        vector.push(0);
        vector
    }

    /// Transform string to \[u16\] with a fixed size. \
    /// Differennt from ygopro, it will keeps 0 for residual part.
    pub fn cast_to_fix_length_array<const N: usize>(message: &str) -> [u16; N] {
        let mut data = [0u16; N];
        for (index, chr) in message.encode_utf16().enumerate() {
            data[index] = chr;
        }
        data
    }

    #[derive(Clone, BinRead, BinWrite)]
    pub struct FixedLengthString<const L: usize> {
        data: [u16; L],
        #[brw(ignore)]
        str: OnceLock<String>
    }

    impl<const L: usize> std::fmt::Display for FixedLengthString<L> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", &cast_to_string(&self.data).unwrap_or("[ERROR]".to_string()))
        }
    }

    impl<const L: usize> PartialEq for FixedLengthString<L> {
        fn eq(&self, other: &Self) -> bool {
            &**self == &**other
        }
    }

    impl <const L: usize> std::fmt::Debug for FixedLengthString<L> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "FixedLengthString[{:}] ", L)?;
            write!(f, "\"{}\"", &cast_to_string(&self.data).unwrap_or("[ERROR]".to_string()))   
        }
    }

    impl<const L: usize> FixedLengthString<L> {
        pub fn allocate() -> Self {
            Self {
                data: [0u16; L],
                str: OnceLock::new(),
            }
        }

        pub fn is_empty(&self) -> bool {
            self.data.iter().all(|&x| x == 0)
        }

        pub fn new(str: String) -> Self {
            let this = Self {
                data: cast_to_fix_length_array(&str),
                str: OnceLock::new()
            };
            this.str.set(str).ok();
            this
        }

        pub fn resolve_data(&mut self) {
            if self.str.get() == None {
                if let Some(str) = cast_to_string(&self.data) {
                    self.str.set(str).ok();
                }
            }
        }

        pub fn resolve_str(&mut self) {
            if let Some(str) = self.str.get() {
                self.data = cast_to_fix_length_array(str);
            }
        }        
    }

    impl<const L: usize> Deref for FixedLengthString<L> {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            self.str.get_or_init(|| cast_to_string(&self.data).unwrap_or_default()).as_str()
        }
    }

    impl<const L: usize> From<String> for FixedLengthString<L> {
        fn from(value: String) -> Self {
            FixedLengthString::new(value)
        }
    }

    impl<'s, const L: usize> From<&'s str> for FixedLengthString<L> {
        fn from(value: &'s str) -> Self {
            FixedLengthString::new(value.to_string())
        }
    }

    #[binrw]
    #[derive(Clone)]
    pub struct U16String {
        #[br(parse_with=until_eof)]
        data: Vec<u16>,
        #[brw(ignore)]
        str: OnceLock<String>,
    }

    impl std::fmt::Debug for U16String {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "U16String[{:}] ", self.data.len())?;
            write!(f, "\"{:}\"", &cast_to_string(&self.data).unwrap_or("[ERROR]".to_string()))
        }
    }
    
    impl std::fmt::Display for U16String {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "U16String[{:}] ", self.data.len())?;
            write!(f, "\"{:}\"", &cast_to_string(&self.data).unwrap_or("[ERROR]".to_string()))
        }
    }

    impl U16String {
        pub fn new(str: String) -> Self {
            let this = Self {
                data: cast_to_c_array(&str),
                str: OnceLock::new()
            };
            this.str.set(str).ok();
            this
        }

        pub fn resolve_data(&self) {
            if self.str.get() == None {
                if let Some(str) = cast_to_string(&self.data) {
                    self.str.set(str).ok();
                }
            }
        }

        pub fn resolve_str(&mut self) {
            if let Some(str) = self.str.get() {
                self.data = cast_to_c_array(str);
            }
        }        
    }

    impl Deref for U16String {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            self.str.get_or_init(|| cast_to_string(&self.data).unwrap_or_default()).as_str()
        }
    }

    impl From<String> for U16String {
        fn from(value: String) -> Self {
            U16String::new(value)
        }
    }

    impl<'s> From<&'s str> for U16String {
        fn from(value: &'s str) -> Self {
            U16String::new(value.to_string())
        }
    }

    impl<'s> From<&'s [u16]> for U16String {
        fn from(value: &'s [u16]) -> Self {
            U16String {
                data: value.to_vec(),
                str: OnceLock::new()
            }
        }
    }
}

pub mod complex {
    use std::io::Cursor;
    use std::io::Write;
    use std::ops::Deref;
    use std::sync::OnceLock;

    use binrw::BinRead;
    use binrw::BinWrite;
    use bytes::Bytes;

    /// Lazy-deserialized message. Holds raw `Bytes` until first access,
    /// then parses into `Message` once and caches the result via `OnceLock`.
    /// When writing, always uses the original raw bytes — never re-serializes.
    #[derive(Debug)]
    pub struct Complex<Message> {
        pub data: Bytes,
        pub message: OnceLock<Message>,
    }

    impl<Message> Clone for Complex<Message> where Message: Clone {
        fn clone(&self) -> Self {
            Self {
                data: self.data.clone(),
                message: self.message.clone(),
            }
        }
    }

    impl<Message: BinRead> Complex<Message> where for<'a> <Message as BinRead>::Args<'a>: Default {
        pub fn new(data: Bytes) -> Self {
            Self {
                data,
                message: OnceLock::new(),
            }
        }

        pub fn from_message(message: Message) -> Self where Message: BinWrite,for<'a> <Message as BinWrite>::Args<'a>: Default {
            let mut cursor = Cursor::new(Vec::new());
            message.write_le(&mut cursor).expect("failed to serialize Complex message");
            Self {
                data: Bytes::from(cursor.into_inner()),
                message: OnceLock::from(message),
            }
        }

        pub fn shadow_clone(&self) -> Self {
            Self {
                data: self.data.clone(),
                message: OnceLock::new()
            }
        }

        pub fn bytes(&self) -> &Bytes {
            &self.data
        }

        pub fn try_get(&self) -> Result<&Message, binrw::Error> {
            if let Some(message) = self.message.get() {
                return Ok(message);
            }
            let message = Message::read_le(&mut Cursor::new(&self.data))?;
            Ok(self.message.get_or_init(|| message))
        }
    }

    impl<Message: BinRead> Deref for Complex<Message> where
        for<'a> <Message as BinRead>::Args<'a>: Default,
    {
        type Target = Message;

        fn deref(&self) -> &Self::Target {
            self.try_get().expect("failed to deserialize Complex message")
        }
    }
    impl<Message: BinWrite> BinWrite for Complex<Message> {
        type Args<'a> = <Message as BinWrite>::Args<'a>;

        fn write_options<W: Write>(&self, writer: &mut W, _endian: binrw::Endian, _args: Self::Args<'_>) -> binrw::BinResult<()> {
            writer.write_all(&self.data).map_err(binrw::Error::from)
        }
    }

    impl<Message> From<Message> for Complex<Message>
    where
        Message: BinRead + BinWrite,
        for<'a> <Message as BinRead>::Args<'a>: Default,
        for<'a> <Message as BinWrite>::Args<'a>: Default,
    {
        fn from(message: Message) -> Self {
            Self::from_message(message)
        }
    }

    impl<Message> From<&Complex<Message>> for crate::message::client_to_server::MessageType {
        fn from(value: &Complex<Message>) -> Self {
            Self::from(value.data[0])
        }
    }
    impl<Message> From<&Complex<Message>> for crate::message::server_to_client::MessageType {
        fn from(value: &Complex<Message>) -> Self {
            Self::from(value.data[0])
        }
    }
    impl<Message> From<&Complex<Message>> for crate::message::game_message::MessageType {
        fn from(value: &Complex<Message>) -> Self {
            Self::from(value.data[0])
        }
    }
}
