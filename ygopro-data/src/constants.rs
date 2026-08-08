#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

use binrw::BinRead;
use binrw::BinWrite;
use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;
use num_enum::FromPrimitive;
use bitflags::bitflags;
use modular_bitfield::bitfield;
use modular_bitfield::Specifier;
use modular_bitfield::error::InvalidBitPattern;
use modular_bitfield::error::OutOfBounds;
use modular_bitfield::specifiers::B3;

use crate::data::DeckErrorType;

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u16)]
#[repr(u16)]
pub enum Network {
    ServerId = 29736,
    ClientId = 57078,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug, PartialOrd, Ord, Hash)]
#[br(map = |raw: u8| Netplayer::from_primitive(raw))]
#[bw(map = |value: &Netplayer| u8::from(*value))]
#[repr(u8)]
pub enum Netplayer {
    Player(u8),
    Observer(u8),
    Unknown = 255
}

impl FromPrimitive for Netplayer {
    type Primitive = u8;

    fn from_primitive(number: Self::Primitive) -> Self {
        if number <= 6 { Netplayer::Player(number) }
        else if number == 7 { Netplayer::Observer(255) }
        else { Netplayer::Unknown }
    }
}

impl From<Netplayer> for u8 {
    fn from(value: Netplayer) -> Self {
        match value {
            Netplayer::Player(index) => index,
            Netplayer::Observer(_) => 7,
            Netplayer::Unknown => 255,
        }
    }
}

impl Specifier for Netplayer {
    const BITS: usize = 4;
    type Bytes = u8;
    type InOut = Self;

    fn into_bytes(input: Self) -> Result<Self::Bytes, OutOfBounds> {
        let byte = u8::from(input);
        if byte >= (1 << Self::BITS) {
            return Err(OutOfBounds);
        }
        Ok(byte)
    }

    fn from_bytes(bytes: Self::Bytes) -> Result<Self, InvalidBitPattern<Self::Bytes>> {
        let netplayer = Netplayer::from_primitive(bytes);
        if netplayer == Netplayer::Unknown {
            return Err(InvalidBitPattern::new(bytes));
        }
        Ok(netplayer)
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug, PartialOrd, Ord, Hash)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum CorePlayer {
    FirstAttackPlayer = 0,
    SecondAttackPlayer = 1,
    None = 2,
    All = 3,
    /// This value is only used as `reason_player` when reason is rule.
    Rule = 5,
}

impl CorePlayer {
    pub fn opponent(&self) -> CorePlayer {
        match *self {
            CorePlayer::FirstAttackPlayer => CorePlayer::SecondAttackPlayer,
            CorePlayer::SecondAttackPlayer => CorePlayer::FirstAttackPlayer,
            CorePlayer::None => CorePlayer::None,
            CorePlayer::All => CorePlayer::All,
            CorePlayer::Rule => CorePlayer::Rule,
        }
    }
}

impl From<Netplayer> for CorePlayer {
    fn from(player: Netplayer) -> Self {
        match player {
            Netplayer::Player(u) => if u % 2 == 0 { CorePlayer::FirstAttackPlayer } else { CorePlayer::SecondAttackPlayer }
            _ => CorePlayer::None
        }
    }
}

impl From<CorePlayer> for Netplayer {
    fn from(player: CorePlayer) -> Self {
        match player {
            CorePlayer::FirstAttackPlayer => Netplayer::Player(0),
            CorePlayer::SecondAttackPlayer => Netplayer::Player(1),
            CorePlayer::None => Netplayer::Unknown,
            CorePlayer::All => Netplayer::Unknown,
            CorePlayer::Rule => Netplayer::Unknown,
        }
    }
}

#[bitfield]
#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug)]
#[br(map = Self::from_bytes)]
#[bw(map = |&x| Self::into_bytes(x))]
#[repr(u8)]
pub struct TypeChange {
    pub player: Netplayer,
    pub host: bool,
    #[skip] __: B3,
}

#[derive(Specifier, Copy, Clone, Eq, PartialEq, Debug)]
#[bits = 4]
#[repr(u8)]
pub enum PlayerChangeState {
    Observe = 0x8,
    Ready = 0x9,
    Notready = 0xa,
    Leave = 0xb,
}

#[bitfield]
#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug)]
#[br(map = Self::from_bytes)]
#[bw(map = |&x| Self::into_bytes(x))]
#[repr(u8)]
pub struct PlayerChange {
    pub state: PlayerChangeState,
    pub player: Netplayer,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr = u8)]
#[repr(u8)]
pub enum JoinError {
    RoomFull = 0,
    WrongPassword = 1,
    HostRefused = 2
}


#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ErrorMessage {
    JoinError(JoinError) = 1,
    DeckError(crate::data::DeckError) = 2,
    SideError = 3,
    VersionError(u16) = 4,
}

fn invalid_data(error: impl std::fmt::Display + Send + Sync + 'static) -> binrw::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into()
}

impl BinRead for ErrorMessage {
    type Args<'a> = ();

    fn read_options<R: std::io::prelude::Read + std::io::prelude::Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::prelude::BinResult<Self> {
        let value = u32::read_options(reader, endian, args)?;
        let code = u32::read_options(reader, endian, args)?;
        let res = match value {
            1 => {
                let err_type = u8::try_from(code).map_err(invalid_data)?;
                ErrorMessage::JoinError(JoinError::try_from(err_type).map_err(|_| invalid_data("invalid JoinError"))?)
            }
            2 => ErrorMessage::DeckError(crate::data::DeckError::from_bytes(code.to_ne_bytes())),
            3 => ErrorMessage::SideError,
            4 => ErrorMessage::VersionError(u16::try_from(code).map_err(invalid_data)?),
            _ => return Err(binrw::Error::NoVariantMatch { pos: 0 }),
        };
        Ok(res)
    }
}

impl BinWrite for ErrorMessage {
    type Args<'a> = ();

    fn write_options<W: std::io::prelude::Write + std::io::prelude::Seek>(&self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> binrw::prelude::BinResult<()> {
        match self {
            ErrorMessage::JoinError(join_error) => {
                u32::write_options(&1, writer, endian, args)?;
                u32::write_options(&(*join_error as u8 as u32), writer, endian, args)?;
            }
            ErrorMessage::DeckError(deck_error) => {
                u32::write_options(&2, writer, endian, args)?;
                u32::write_options(&u32::from(*deck_error), writer, endian, args)?;
            },
            ErrorMessage::SideError => {
                u32::write_options(&3, writer, endian, args)?;
                u32::write_options(&0, writer, endian, args)?;
            },
            ErrorMessage::VersionError(version) => {
                u32::write_options(&4, writer, endian, args)?;
                u32::write_options(&(*version as u32), writer, endian, args)?;
            },
        }
        Ok(())
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug, Hash)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum Mode {
    Single = 0,
    Match = 1,
    Tag = 2,
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Location: u8 {
        const Limbo = 0;
        const Deck = 0x1;
        const Hand = 0x2;
        const MZone = 0x4;
        const SZone = 0x8;
        const Grave = 0x10;
        const Removed = 0x20;
        const Extra = 0x40;
        const Overlay = 0x80;
        const OnField = 0xc;
        // FZone = 0x100,
        // PZone = 0x200,
        // DeckBot = 0x10001,
        // DeckShf = 0x20001,
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Position: u8 {
        const Any = 0;
        const FaceupAttack = 0x1;
        const FaceDownAttack = 0x2;
        const FaceupDefense = 0x4;
        const FacedownDefense = 0x8;
        const Faceup = 0x5;
        const Facedown = 0xa;
        const Attack = 0x3;
        const Defense = 0xc;
        // NoFlipEffect = 0x10000
    }
}

impl Position {
    pub fn is_face_down(&self) -> bool {
        self.intersects(Position::Facedown)
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Timing: u32 {
        const DrawPhase = 0x1;
        const StandbyPhase = 0x2;
        const MainEnd = 0x4;
        const BattleStart = 0x8;
        const BattleEnd = 0x10;
        const EndPhase = 0x20;
        const Summon = 0x40;
        const SpecialSummon = 0x80;
        const FlipSummon = 0x100;
        const MonsterSet = 0x200;
        const SpellTrapSet = 0x400;
        const PositionChange = 0x800;
        const Attack = 0x1000;
        const DamageStep = 0x2000;
        const DamageCalculate = 0x4000;
        const ChainEnd = 0x8000;
        const Draw = 0x10000;
        const Damage = 0x20000;
        const Recover = 0x40000;
        const Destroy = 0x80000;
        const Remove = 0x100000;
        const ToHand = 0x200000;
        const ToDeck = 0x400000;
        const ToGrave = 0x800000;
        const BattlePhase = 0x1000000;
        const Equip = 0x2000000;
        const BattleStepEnd = 0x4000000;
        const Battled = 0x8000000;
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Type: u32 {
        const Monster = 0x1;
        const Spell = 0x2;
        const Trap = 0x4;
        const Normal = 0x10;
        const Effect = 0x20;
        const Fusion = 0x40;
        const Ritual = 0x80;
        const Trapmonster = 0x100;
        const Spirit = 0x200;
        const Union = 0x400;
        const Dual = 0x800;
        const Tuner = 0x1000;
        const Synchro = 0x2000;
        const Token = 0x4000;
        const Quickplay = 0x10000;
        const Continuous = 0x20000;
        const Equip = 0x40000;
        const Field = 0x80000;
        const Counter = 0x100000;
        const Flip = 0x200000;
        const Toon = 0x400000;
        const Xyz = 0x800000;
        const Pendulum = 0x1000000;
        const SpecialSummon = 0x2000000;
        const Link = 0x4000000;
        const ExtraDeck = 0x4802040;
    }
}


bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Race: u32 {
        const Warrior = 0x1;
        const Spellcaster = 0x2;
        const Fairy = 0x4;
        const Fiend = 0x8;
        const Zombie = 0x10;
        const Machine = 0x20;
        const Aqua = 0x40;
        const Pyro = 0x80;
        const Rock = 0x100;
        const Windbeast = 0x200;
        const Plant = 0x400;
        const Insect = 0x800;
        const Thunder = 0x1000;
        const Dragon = 0x2000;
        const Beast = 0x4000;
        const Beastwarrior = 0x8000;
        const Dinosaur = 0x10000;
        const Fish = 0x20000;
        const Seaserpent = 0x40000;
        const Reptile = 0x80000;
        const Psycho = 0x100000;
        const Devine = 0x200000;
        const Creatorgod = 0x400000;
        const Wyrm = 0x800000;
        const Cyberse = 0x1000000;
        const Illusion = 0x2000000;
        const All = 0x3ffffff;
    }
}

impl Race {
    pub const COUNT: u8 = 26;
}


bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Reason: u32 {
        const Destroy = 0x1;
        const Release = 0x2;
        const Temporary = 0x4;
        const Material = 0x8;
        const Summon = 0x10;
        const Battle = 0x20;
        const Effect = 0x40;
        const Cost = 0x80;
        const Adjust = 0x100;
        const LostTarget = 0x200;
        const Rule = 0x400;
        const SpecialSummon = 0x800;
        const DisableSummon = 0x1000;
        const Flip = 0x2000;
        const Discard = 0x4000;
        const RecoverDamage = 0x8000;
        const RecoverRecover = 0x10000;
        const Return = 0x20000;
        const Fusion = 0x40000;
        const Synchro = 0x80000;
        const Ritual = 0x100000;
        const Xyz = 0x200000;
        const Replace = 0x1000000;
        const Draw = 0x2000000;
        const Redirect = 0x4000000;
        const Reveal = 0x8000000;
        const Link = 0x10000000;
        const LostOverlay = 0x20000000;
        const Maintenance = 0x40000000;
        const Action = 0x80000000;
        const Procedure = 0x10280000;
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Status: u32 {
        const Disabled = 0x0001;
        const ToEnable = 0x0002;
        const ToDisable = 0x0004;
        const ProcessComplete = 0x0008;
        const SetTurn = 0x0010;
        const NoLevel = 0x0020;
        const BattleResult = 0x0040;
        const SpecialSummonStep = 0x0080;
        const CannotChangeForm = 0x0100;
        const Summoning = 0x0200;
        const EffectEnabled = 0x0400;
        const SummonTurn = 0x0800;
        const DestroyConfirmed = 0x1000;
        const LeaveConfirmed = 0x2000;
        const BattleDestroyed = 0x4000;
        const CopyingEffect = 0x8000;
        const Chaining = 0x10000;
        const SummonDisabled = 0x20000;
        const ActivateDisabled = 0x40000;
        const EffectReplaced = 0x80000;
        const FlipSummoning = 0x100000;
        const AttackCanceled = 0x200000;
        const Initializing = 0x400000;
        const ToHandWithoutConfirm = 0x800000;
        const JustPos = 0x1000000;
        const ContinuousPos = 0x2000000;
        const Forbidden = 0x4000000;
        const ActFromHand = 0x8000000;
        const OpponentBattle = 0x10000000;
        const FlipSummonTurn = 0x20000000;
        const SpecialSummonTurn = 0x40000000;
        const FlipSummonDisabled = 0x80000000;
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Query: u32 {
        const Code = 0x1;
        const Position = 0x2;
        const Alias = 0x4;
        const Type = 0x8;
        const Level = 0x10;
        const Rank = 0x20;
        const Attribute = 0x40;
        const Race = 0x80;
        const Attack = 0x100;
        const Defense = 0x200;
        const BaseAttack = 0x400;
        const BaseDefense = 0x800;
        const Reason = 0x1000;
        const ReasonCard = 0x2000;
        const EquipCard = 0x4000;
        const TargetCard = 0x8000;
        const OverlayCard = 0x10000;
        const Counters = 0x20000;
        const Owner = 0x40000;
        const Status = 0x80000;
        const LeftScale = 0x200000;
        const RightScale = 0x400000;
        const Link = 0x800000;
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Attribute: u32 {
        const Earth = 0x1;
        const Water = 0x2;
        const Fire = 0x4;
        const Wind = 0x8;
        const Light = 0x10;
        const Dark = 0x20;
        const Devine = 0x40;
        const All = 0x7f;
    }
}

impl Attribute {
    pub const COUNT: u8 = 7;
}


bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Linkmarkers: u32 {
        const BottomLeft = 0x1;
        const Bottom = 0x2;
        const BottomRight = 0x4;
        const Left = 0x8;
        const Right = 0x20;
        const TopLeft = 0x40;
        const Top = 0x80;
        const TopRight = 0x100;
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum DuelStage {
    Begin = 0,
    Finger = 1,
    Firstgo = 2,
    Dueling = 3,
    Siding = 4,
    End = 5,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum Color {
    Observer = 7,
    Lightblue = 8,
    Red = 11,
    Green = 12,
    Blue = 13,
    Babyblue = 14,
    Pink = 15,
    Yellow = 16,
    White = 17,
    Gray = 18,
    Darkgray = 19,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum Hint {
    Event = 1,
    Message = 2,
    SelectMessage = 3,
    OpponentSelected = 4,
    Effect = 5,
    Race = 6,
    Attribute = 7,
    Code = 8,
    Number = 9,
    Card = 10,
    Zone = 11,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u16)]
#[repr(u16)]
pub enum Phase {
    Draw = 0x1,
    Standby = 0x2,
    Main1 = 0x4,
    BattleStart = 0x8,
    BattleStep = 0x10,
    Damage = 0x20,
    DamageCalculate = 0x40,
    Battle = 0x80,
    Main2 = 0x100,
    End = 0x200,
}


bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct SummonType: u32 {
        const Normal = 0x10000000;
        const Advance = 0x11000000;
        const Dual = 0x12000000;
        const Flip = 0x20000000;
        const Special = 0x40000000;
        const Fusion = 0x43000000;
        const Ritual = 0x45000000;
        const Synchro = 0x46000000;
        const Xyz = 0x49000000;
        const Pendulum = 0x4a000000;
        const Link = 0x4c000000;
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum Hand {
    Scissors = 1,
    Rock = 2,
    Paper = 3
}

#[derive(PartialEq, Eq)]
pub enum HandResult {
    Win,
    Draw,
    Lose
}

impl Hand {
    pub fn judge(&self, other: &Self) -> HandResult {
        if self == other { return HandResult::Draw; }
        match self {
            Hand::Scissors => if *other == Hand::Paper { HandResult::Win } else { HandResult::Lose },
            Hand::Rock => if *other == Hand::Scissors { HandResult::Win } else { HandResult::Lose },
            Hand::Paper => if *other == Hand::Rock { HandResult::Win } else { HandResult::Lose },
        }
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Eq, PartialEq, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct OT: u8 {
        const OCG = 1;
        const TCG = 2;
        const Custom = 4;
        const SC = 8;
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr = u8)]
#[repr(u8)]
pub enum Rule {
    OCG = 0,
    TCG = 1,
    SC = 2,
    Custom = 3,
    OCG_TCG = 4,
    All = 5,
}

impl From<Rule> for OT {
    fn from(rule: Rule) -> Self {
        match rule {
            Rule::OCG => OT::OCG,
            Rule::TCG => OT::TCG,
            Rule::SC => OT::SC,
            Rule::Custom => OT::Custom,
            Rule::OCG_TCG => OT::OCG | OT::TCG,
            Rule::All => OT::empty(),
        }
    }
}

impl Rule {
    pub fn check_ot(&self, ot: OT) -> Option<DeckErrorType> {
        let allowed = OT::from(*self);
        if ot.contains(allowed) { return None; }
        if ot.contains(OT::OCG) && allowed != OT::OCG {
            return Some(DeckErrorType::OcgOnly);
        }
        if ot.contains(OT::TCG) && allowed != OT::TCG {
            return Some(DeckErrorType::TcgOnly);
        }
        Some(DeckErrorType::NotAvailable)
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(BinRead, BinWrite, Clone, Copy, Default, Debug)]
    #[br(map=|x| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct Category: u32 {
        const Destroy = 0x1;
        const Release = 0x2;
        const Remove = 0x4;
        const ToHand = 0x8;
        const ToDeck = 0x10;
        const ToGrave = 0x20;
        const DeckDestroy = 0x40;
        const HandDestroy = 0x80;
        const Summon = 0x100;
        const SpecialSummon = 0x200;
        const Token = 0x400;
        const GraveAction = 0x800;
        const Position = 0x1000;
        const Control = 0x2000;
        const Disable = 0x4000;
        const DisableSummon = 0x8000;
        const Draw = 0x10000;
        const Search = 0x20000;
        const Equip = 0x40000;
        const Damage = 0x80000;
        const Recover = 0x100000;
        const AttackChange = 0x200000;
        const DefenseChange = 0x400000;
        const Counter = 0x800000;
        const Coin = 0x1000000;
        const Dice = 0x2000000;
        const LeaveGrave = 0x4000000;
        const GraveSpecialSummon = 0x8000000;
        const Negate = 0x10000000;
        const Announce = 0x20000000;
        const FusionSummon = 0x40000000;
        const ToExtra = 0x80000000;
    }
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, FromPrimitive, IntoPrimitive, Debug)]
#[br(map = |v: u32| Operation::from(v))]
#[bw(map = |v: &Operation| u32::from(*v))]
#[repr(u32)]
pub enum Operation {
    Add = 0x40000000,
    Subtract = 0x40000001,
    Multiply = 0x40000002,
    Divide = 0x40000003,
    And = 0x40000004,
    Or  = 0x40000005,
    Negate = 0x40000006,
    Not = 0x40000007,
    IsCode = 0x40000100,
    IsSetcard = 0x40000101,
    IsType = 0x40000102,
    IsRace = 0x40000103,
    IsAttribute = 0x40000104,
    #[num_enum(catch_all)]
    Operand(u32)
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum MasterRule {
    MasterRule1 = 1,
    MasterRule2 = 2,
    MasterRule3 = 3,
    MasterRuleNew = 4,
    MasterRule2020 = 5,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum Activity {
    Summon = 1,
    NormalSummon = 2,
    SpecialSummon = 3,
    FlipSummon = 4,
    Attack = 5,
    BattlePhase = 6,
    Chain = 7,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum CardHint {
    Turn = 1,
    Card = 2,
    Race = 3,
    Attribute = 4,
    Number = 5,
    DescriptionAdd = 6,
    DescriptionRemove = 7,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum PlayerHint {
    DescriptionAdd = 6,
    DescriptionRemove = 7,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=u8)]
#[repr(u8)]
pub enum EffectDescription {
    Operation = 1,
    Reset = 2,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr=i8)]
#[repr(i8)]
pub enum OperationResult {
    Canceled = -1,
    Fail = 0,
    Success = 1,
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, FromPrimitive, IntoPrimitive, Debug)]
#[br(map = |v: u8| WinReason::from(v))]
#[bw(map = |v: &WinReason| u8::from(*v))]
#[repr(u8)]
pub enum WinReason {
    OpponentSurrender = 0,
    LPZero = 1,
    DeckOut = 2,
    Timeout = 3,
    OpponentLeave = 4,
    #[num_enum(catch_all)]
    Other(u8)
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, TryFromPrimitive, IntoPrimitive, Debug)]
#[brw(repr = i8)]
#[repr(i8)]
pub enum SelectSumMode {
    Exact = 0,
    AtLeast = 1,
}
