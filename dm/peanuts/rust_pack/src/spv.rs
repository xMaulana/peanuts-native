//! SPIR-V module parser, indexer, and Android Vulkan lowering passes.
//!
//! Mirrors the semantics and output of `dm/gfx/tools/vk_android_spv.py` while
//! storing instructions as compact `u32` vectors instead of Python objects.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const MAGIC: u32 = 0x0723_0203;

// Opcodes
pub const OP_NAME: u16 = 5;
pub const OP_EXTENSION: u16 = 10;
pub const OP_EXT_INST_IMPORT: u16 = 11;
pub const OP_EXT_INST: u16 = 12;
pub const OP_MEMORY_MODEL: u16 = 14;
pub const OP_ENTRY_POINT: u16 = 15;
pub const OP_EXECUTION_MODE: u16 = 16;
pub const OP_CAPABILITY: u16 = 17;

pub const OP_TYPE_VOID: u16 = 19;
pub const OP_TYPE_BOOL: u16 = 20;
pub const OP_TYPE_INT: u16 = 21;
pub const OP_TYPE_FLOAT: u16 = 22;
pub const OP_TYPE_VECTOR: u16 = 23;
pub const OP_TYPE_IMAGE: u16 = 25;
pub const OP_TYPE_SAMPLER: u16 = 26;
pub const OP_TYPE_SAMPLED_IMAGE: u16 = 27;
pub const OP_TYPE_ARRAY: u16 = 28;
pub const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
pub const OP_TYPE_STRUCT: u16 = 30;
pub const OP_TYPE_POINTER: u16 = 32;

pub const OP_CONSTANT: u16 = 43;
pub const OP_CONSTANT_COMPOSITE: u16 = 44;
pub const OP_SPEC_CONSTANT_FALSE: u16 = 49;
pub const OP_FUNCTION: u16 = 54;
pub const OP_FUNCTION_END: u16 = 56;
pub const OP_VARIABLE: u16 = 59;
pub const OP_LOAD: u16 = 61;
pub const OP_STORE: u16 = 62;
pub const OP_ACCESS_CHAIN: u16 = 65;
pub const OP_IN_BOUNDS_ACCESS_CHAIN: u16 = 66;
pub const OP_DECORATE: u16 = 71;
pub const OP_MEMBER_DECORATE: u16 = 72;
pub const OP_COMPOSITE_EXTRACT: u16 = 81;
pub const OP_COMPOSITE_INSERT: u16 = 82;
pub const OP_COPY_OBJECT: u16 = 83;
pub const OP_SAMPLED_IMAGE: u16 = 86;
pub const OP_IMAGE_READ: u16 = 98;
pub const OP_IMAGE_WRITE: u16 = 99;
pub const OP_IMAGE: u16 = 100;
pub const OP_IMAGE_QUERY_SIZE_LOD: u16 = 103;
pub const OP_IMAGE_QUERY_SIZE: u16 = 104;
pub const OP_FMUL: u16 = 133;
pub const OP_SELECT: u16 = 169;
pub const OP_EMIT_VERTEX: u16 = 218;
pub const OP_EMIT_STREAM_VERTEX: u16 = 220;
pub const OP_LABEL: u16 = 248;
pub const OP_RETURN: u16 = 253;
pub const OP_RETURN_VALUE: u16 = 254;

// Decorations, storage classes, builtins
pub const DEC_ARRAY_STRIDE: u32 = 6;
pub const DEC_SPEC_ID: u32 = 1;
pub const DEC_BUILTIN: u32 = 11;
pub const DEC_LOCATION: u32 = 30;
pub const DEC_INDEX: u32 = 32;
pub const DEC_BINDING: u32 = 33;
pub const DEC_SET: u32 = 34;
pub const DEC_NON_UNIFORM: u32 = 5300;

pub const SC_UNIFORM_CONSTANT: u32 = 0;
pub const SC_INPUT: u32 = 1;
pub const SC_OUTPUT: u32 = 3;
pub const SC_WORKGROUP: u32 = 4;

pub const BUILTIN_POSITION: u32 = 0;
pub const DIM_BUFFER: u32 = 5;

// Capabilities
pub const CAP_FLOAT64: u32 = 10;
pub const CAP_STORAGE_EXTENDED: u32 = 49;
pub const CAP_READ_WITHOUT_FORMAT: u32 = 55;
pub const CAP_WRITE_WITHOUT_FORMAT: u32 = 56;
pub const CAP_SHADER_NON_UNIFORM: u32 = 5301;
pub const CAP_SAMPLED_IMAGE_ARRAY_NU: u32 = 5307;
pub const CAP_STORAGE_IMAGE_ARRAY_NU: u32 = 5309;

const INTERLOCK_OPS: &[u16] = &[5364, 5365];
const INTERLOCK_CAPS: &[u32] = &[5363, 5372, 5378];
const INTERLOCK_MODES: &[u32] = &[5366, 5367, 5368, 5369, 5370, 5371];
const SUBGROUP_BUILTINS: &[u32] = &[36, 37, 40, 41, 4416, 4417, 4418, 4419, 4420];
const SUBGROUP_CAPS: &[u32] = &[61, 62, 63, 64, 65, 66, 67, 68, 4423, 4431];
const BASE_IMAGE_FORMATS: &[u32] = &[0, 1, 2, 3, 4, 5, 21, 22, 23, 24, 30, 31, 32, 33];

#[inline]
pub fn opcode(inst: &[u32]) -> u16 {
    (inst[0] & 0xffff) as u16
}

#[inline]
fn make_word0(len: usize, op: u16) -> u32 {
    ((len as u32) << 16) | (op as u32)
}

pub fn string_at(words: &[u32], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < words.len() {
        let b = words[i].to_le_bytes();
        i += 1;
        if let Some(pos) = b.iter().position(|&c| c == 0) {
            out.extend_from_slice(&b[..pos]);
            break;
        }
        out.extend_from_slice(&b);
    }
    (String::from_utf8_lossy(&out).into_owned(), i)
}

pub fn lit_string(text: &str) -> Vec<u32> {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn is_subgroup_op(op: u16) -> bool {
    (333..367).contains(&op) || matches!(op, 4421 | 4422 | 4428 | 4429 | 4430 | 4432)
}

#[derive(Clone)]
pub struct SpvModule {
    pub header: [u32; 5],
    pub ins: Vec<Vec<u32>>,
    pub names: HashMap<u32, String>,
    pub decos: HashMap<u32, HashMap<u32, Vec<u32>>>,
    pub types: HashMap<u32, Vec<u32>>,
    pub consts: HashMap<u32, (u32, Vec<u32>)>,
    pub vars: HashMap<u32, (u32, u32)>,
    pub caps: Vec<u32>,
    pub exts: Vec<String>,
    pub entry: Option<Vec<u32>>,
    pub modes: Vec<Vec<u32>>,
}

impl SpvModule {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() % 4 != 0 || data.len() < 20 {
            return Err("not a SPIR-V module (length)".to_string());
        }
        let words: Vec<u32> = data
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if words[0] != MAGIC {
            return Err("not a SPIR-V module (magic)".to_string());
        }
        let header = [words[0], words[1], words[2], words[3], words[4]];
        let mut ins = Vec::with_capacity(words.len() / 4);
        let mut i = 5usize;
        while i < words.len() {
            let n = (words[i] >> 16) as usize;
            if n == 0 || i + n > words.len() {
                return Err(format!("malformed SPIR-V instruction at word {i}"));
            }
            ins.push(words[i..i + n].to_vec());
            i += n;
        }
        let mut m = Self {
            header,
            ins,
            names: HashMap::new(),
            decos: HashMap::new(),
            types: HashMap::new(),
            consts: HashMap::new(),
            vars: HashMap::new(),
            caps: Vec::new(),
            exts: Vec::new(),
            entry: None,
            modes: Vec::new(),
        };
        m.index();
        Ok(m)
    }

    pub fn index(&mut self) {
        self.names.clear();
        self.decos.clear();
        self.types.clear();
        self.consts.clear();
        self.vars.clear();
        self.caps.clear();
        self.exts.clear();
        self.entry = None;
        self.modes.clear();

        for x in &self.ins {
            let op = opcode(x);
            if op == OP_NAME && x.len() >= 3 {
                self.names.insert(x[1], string_at(x, 2).0);
            } else if op == OP_DECORATE && x.len() >= 3 {
                self.decos
                    .entry(x[1])
                    .or_default()
                    .insert(x[2], x[3..].to_vec());
            } else if op == OP_CAPABILITY && x.len() >= 2 {
                self.caps.push(x[1]);
            } else if op == OP_EXTENSION && x.len() >= 2 {
                self.exts.push(string_at(x, 1).0);
            } else if op == OP_ENTRY_POINT {
                self.entry = Some(x.clone());
            } else if op == OP_EXECUTION_MODE && x.len() >= 3 {
                self.modes.push(x[2..].to_vec());
            } else if (19..=39).contains(&op) && x.len() >= 2 {
                self.types.insert(x[1], x.clone());
            } else if op == OP_CONSTANT && x.len() >= 4 {
                self.consts.insert(x[2], (x[1], x[3..].to_vec()));
            } else if op == OP_VARIABLE && x.len() >= 4 {
                self.vars.insert(x[2], (x[1], x[3]));
            }
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let total_words = 5 + self.ins.iter().map(|x| x.len()).sum::<usize>();
        let mut out = Vec::with_capacity(total_words * 4);
        for &w in &self.header {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for inst in &self.ins {
            for &w in inst {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        out
    }

    #[inline]
    pub fn new_id(&mut self) -> u32 {
        let id = self.header[3];
        self.header[3] += 1;
        id
    }

    #[inline]
    pub fn op(&self, ident: u32) -> Option<u16> {
        self.types.get(&ident).map(|x| opcode(x))
    }

    #[inline]
    pub fn pointee(&self, ptr: u32) -> Option<(u32, u32)> {
        self.types
            .get(&ptr)
            .filter(|t| t.len() >= 4)
            .map(|t| (t[2], t[3]))
    }

    pub fn strip_arrays(&self, mut t: u32) -> (u32, u32) {
        let mut count = 1u32;
        while let Some(op) = self.op(t) {
            if op != OP_TYPE_ARRAY && op != OP_TYPE_RUNTIME_ARRAY {
                break;
            }
            let x = &self.types[&t];
            if op == OP_TYPE_RUNTIME_ARRAY {
                count = 0;
                t = x[2];
                continue;
            }
            let n = self
                .consts
                .get(&x[3])
                .and_then(|(_, v)| v.first().copied())
                .unwrap_or(0);
            count = count.saturating_mul(n);
            t = x[2];
        }
        (t, count)
    }

    pub fn find_type(&self, op: u16, operands: &[u32]) -> Option<u32> {
        for (&id, x) in &self.types {
            if opcode(x) == op && x.len() >= 2 && &x[2..] == operands {
                return Some(id);
            }
        }
        None
    }

    fn insert_decl(&mut self, inst: Vec<u32>) {
        let at = self
            .ins
            .iter()
            .position(|y| opcode(y) == OP_FUNCTION)
            .unwrap_or(self.ins.len());
        self.ins.insert(at, inst);
    }

    pub fn add_type(&mut self, op: u16, operands: &[u32]) -> u32 {
        if let Some(id) = self.find_type(op, operands) {
            return id;
        }
        let id = self.new_id();
        let mut inst = Vec::with_capacity(2 + operands.len());
        inst.push(make_word0(2 + operands.len(), op));
        inst.push(id);
        inst.extend_from_slice(operands);
        self.insert_decl(inst.clone());
        self.types.insert(id, inst);
        id
    }

    pub fn add_const(&mut self, ty: u32, value: u32) -> u32 {
        for (&id, (t, v)) in &self.consts {
            if *t == ty && v.len() == 1 && v[0] == value {
                return id;
            }
        }
        let id = self.new_id();
        let inst = vec![make_word0(4, OP_CONSTANT), ty, id, value];
        self.insert_decl(inst);
        self.consts.insert(id, (ty, vec![value]));
        id
    }

    pub fn entry_model(&self) -> Option<&'static str> {
        let ep = self.entry.as_ref()?;
        match ep.get(1).copied()? {
            0 => Some("vs"),
            1 => Some("hs"),
            2 => Some("ds"),
            3 => Some("gs"),
            4 => Some("ps"),
            5 => Some("cs"),
            5364 => Some("ts"),
            5365 => Some("ms"),
            _ => None,
        }
    }

    pub fn interface(&self) -> &[u32] {
        if let Some(ep) = &self.entry {
            let (_, k) = string_at(ep, 3);
            if k <= ep.len() {
                return &ep[k..];
            }
        }
        &[]
    }

    pub fn io(&self, sc: u32) -> BTreeMap<u32, (u32, u32)> {
        let mut out = BTreeMap::new();
        for &vid in self.interface() {
            if let Some(&(ptr, c)) = self.vars.get(&vid) {
                if c != sc {
                    continue;
                }
                if let Some(d) = self.decos.get(&vid) {
                    if let Some(loc_vals) = d.get(&DEC_LOCATION) {
                        if let Some(&loc) = loc_vals.first() {
                            if let Some((_, val_ty)) = self.pointee(ptr) {
                                out.insert(loc, (vid, val_ty));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn builtin_var(&self, sc: u32, builtin: u32) -> Option<(u32, Option<u32>)> {
        for &vid in self.interface() {
            let Some(&(ptr, c)) = self.vars.get(&vid) else {
                continue;
            };
            if c != sc {
                continue;
            }
            if self
                .decos
                .get(&vid)
                .and_then(|d| d.get(&DEC_BUILTIN))
                .map(|v| v.as_slice() == [builtin])
                .unwrap_or(false)
            {
                return Some((vid, None));
            }
            if let Some((_, t)) = self.pointee(ptr) {
                for x in &self.ins {
                    if opcode(x) == OP_MEMBER_DECORATE
                        && x.len() >= 5
                        && x[1] == t
                        && x[3] == DEC_BUILTIN
                        && x[4] == builtin
                    {
                        return Some((vid, Some(x[2])));
                    }
                }
            }
        }
        None
    }

    pub fn function_range(&self, fid: u32) -> Option<(usize, usize)> {
        let a = self
            .ins
            .iter()
            .position(|x| opcode(x) == OP_FUNCTION && x.get(2) == Some(&fid))?;
        let e = (a..self.ins.len()).find(|&k| opcode(&self.ins[k]) == OP_FUNCTION_END)?;
        Some((a, e))
    }

    pub fn first_block_insert_point(&self, fid: u32) -> Option<usize> {
        let (a, _) = self.function_range(fid)?;
        let mut k = (a..self.ins.len()).find(|&idx| opcode(&self.ins[idx]) == OP_LABEL)? + 1;
        while k < self.ins.len() && opcode(&self.ins[k]) == OP_VARIABLE {
            k += 1;
        }
        Some(k)
    }

    pub fn set_binding(&mut self, vid: u32, s: u32, b: u32) {
        for x in &mut self.ins {
            if opcode(x) == OP_DECORATE && x.len() >= 4 && x[1] == vid {
                if x[2] == DEC_SET {
                    x[3] = s;
                } else if x[2] == DEC_BINDING {
                    x[3] = b;
                }
            }
        }
        let has_set = self
            .decos
            .get(&vid)
            .map(|d| d.contains_key(&DEC_SET))
            .unwrap_or(false);
        if !has_set {
            if let Some(at) = self
                .ins
                .iter()
                .position(|y| opcode(y) == OP_DECORATE && y.get(1) == Some(&vid))
            {
                self.ins
                    .insert(at, vec![make_word0(4, OP_DECORATE), vid, DEC_SET, s]);
            }
        }
        let d = self.decos.entry(vid).or_default();
        d.insert(DEC_SET, vec![s]);
        d.insert(DEC_BINDING, vec![b]);
    }

    pub fn subgroup_used(&self) -> bool {
        if self.ins.iter().any(|x| is_subgroup_op(opcode(x))) {
            return true;
        }
        for d in self.decos.values() {
            if let Some(v) = d.get(&DEC_BUILTIN) {
                if v.first().map(|b| SUBGROUP_BUILTINS.contains(b)).unwrap_or(false) {
                    return true;
                }
            }
        }
        self.ins.iter().any(|x| {
            opcode(x) == OP_MEMBER_DECORATE
                && x.len() >= 5
                && x[3] == DEC_BUILTIN
                && SUBGROUP_BUILTINS.contains(&x[4])
        })
    }

    pub fn strip_unused_caps(&mut self) -> Vec<u32> {
        let mut drop_set = HashSet::new();
        if !self
            .ins
            .iter()
            .any(|x| opcode(x) == OP_TYPE_FLOAT && x.get(2) == Some(&64))
        {
            drop_set.insert(CAP_FLOAT64);
        }
        if !self.ins.iter().any(|x| INTERLOCK_OPS.contains(&opcode(x))) {
            for &c in INTERLOCK_CAPS {
                drop_set.insert(c);
            }
        }
        if !self.subgroup_used() {
            for &c in SUBGROUP_CAPS {
                drop_set.insert(c);
            }
        }
        let present: HashSet<u32> = self.caps.iter().copied().collect();
        drop_set.retain(|c| present.contains(c));
        let drop_interlock = INTERLOCK_CAPS.iter().any(|c| drop_set.contains(c));

        self.ins.retain(|x| {
            let op = opcode(x);
            if op == OP_CAPABILITY && x.get(1).map(|c| drop_set.contains(c)).unwrap_or(false) {
                return false;
            }
            if op == OP_EXTENSION
                && drop_interlock
                && string_at(x, 1).0 == "SPV_EXT_fragment_shader_interlock"
            {
                return false;
            }
            if op == OP_EXECUTION_MODE
                && drop_interlock
                && x.get(2).map(|m| INTERLOCK_MODES.contains(m)).unwrap_or(false)
            {
                return false;
            }
            true
        });
        self.index();
        let mut dropped: Vec<u32> = drop_set.into_iter().collect();
        dropped.sort_unstable();
        dropped
    }

    pub fn formatless_storage_caps(&mut self) -> Vec<u32> {
        let imgs: HashSet<u32> = self
            .types
            .iter()
            .filter_map(|(&id, x)| {
                if opcode(x) == OP_TYPE_IMAGE && x.len() >= 9 && x[7] == 2 && x[8] == 0 {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        if imgs.is_empty() {
            return Vec::new();
        }
        let typed: HashSet<u32> = self
            .ins
            .iter()
            .filter_map(|x| {
                if opcode(x) == OP_LOAD && x.len() >= 3 && imgs.contains(&x[1]) {
                    Some(x[2])
                } else {
                    None
                }
            })
            .collect();
        let mut need = BTreeSet::new();
        for x in &self.ins {
            let op = opcode(x);
            if op == OP_IMAGE_READ && x.len() >= 4 && typed.contains(&x[3]) {
                need.insert(CAP_READ_WITHOUT_FORMAT);
            } else if op == OP_IMAGE_WRITE && x.len() >= 2 && typed.contains(&x[1]) {
                need.insert(CAP_WRITE_WITHOUT_FORMAT);
            }
        }
        for &c in &self.caps {
            need.remove(&c);
        }
        if need.is_empty() {
            return Vec::new();
        }
        let at = self
            .ins
            .iter()
            .rposition(|y| opcode(y) == OP_CAPABILITY)
            .map(|p| p + 1)
            .unwrap_or(0);
        for &c in &need {
            self.ins.insert(at, vec![make_word0(2, OP_CAPABILITY), c]);
        }
        self.index();
        need.into_iter().collect()
    }

    pub fn extended_format_caps(&mut self) -> Vec<u32> {
        if self.caps.contains(&CAP_STORAGE_EXTENDED) {
            return Vec::new();
        }
        let needs = self.types.values().any(|x| {
            opcode(x) == OP_TYPE_IMAGE
                && x.len() >= 9
                && x[8] < 40
                && !BASE_IMAGE_FORMATS.contains(&x[8])
        });
        if !needs {
            return Vec::new();
        }
        let at = self
            .ins
            .iter()
            .rposition(|y| opcode(y) == OP_CAPABILITY)
            .map(|p| p + 1)
            .unwrap_or(0);
        self.ins
            .insert(at, vec![make_word0(2, OP_CAPABILITY), CAP_STORAGE_EXTENDED]);
        self.index();
        vec![CAP_STORAGE_EXTENDED]
    }

    pub fn workgroup_plain_arrays(&mut self) -> u32 {
        let mut done = 0u32;
        let mut made: HashMap<u32, u32> = HashMap::new();
        let vars: Vec<(u32, u32, u32)> = self
            .vars
            .iter()
            .map(|(&vid, &(ptr, sc))| (vid, ptr, sc))
            .collect();
        for (vid, ptr, sc) in vars {
            if sc != SC_WORKGROUP {
                continue;
            }
            let Some((_, t)) = self.pointee(ptr) else {
                continue;
            };
            if self.op(t) != Some(OP_TYPE_ARRAY) {
                continue;
            }
            let has_stride = self
                .decos
                .get(&t)
                .map(|d| d.contains_key(&DEC_ARRAY_STRIDE))
                .unwrap_or(false);
            if !has_stride {
                continue;
            }
            let Some(mut k) = self
                .ins
                .iter()
                .position(|x| opcode(x) == OP_VARIABLE && x.get(2) == Some(&vid))
            else {
                continue;
            };
            if !made.contains_key(&t) {
                let arr = self.new_id();
                let p = self.new_id();
                let t_inst = self.types[&t].clone();
                let arr_inst = vec![make_word0(4, OP_TYPE_ARRAY), arr, t_inst[2], t_inst[3]];
                let ptr_inst = vec![make_word0(4, OP_TYPE_POINTER), p, sc, arr];
                self.ins.splice(k..k, [arr_inst, ptr_inst]);
                made.insert(t, p);
                k += 2;
            }
            self.ins[k][1] = made[&t];
            done += 1;
        }
        if done > 0 {
            self.index();
        }
        done
    }

    pub fn precise_minmax(&mut self) -> u32 {
        let glsl: HashSet<u32> = self
            .ins
            .iter()
            .filter_map(|x| {
                if opcode(x) == OP_EXT_INST_IMPORT && string_at(x, 2).0 == "GLSL.std.450" {
                    Some(x[1])
                } else {
                    None
                }
            })
            .collect();
        let mut n = 0u32;
        for x in &mut self.ins {
            if opcode(x) == OP_EXT_INST && x.len() >= 5 && glsl.contains(&x[3]) {
                let new_op = match x[4] {
                    37 => Some(79),
                    40 => Some(80),
                    43 => Some(81),
                    _ => None,
                };
                if let Some(op2) = new_op {
                    let ty = x[1];
                    let is_int = self.types.get(&ty).map(|t| opcode(t) == OP_TYPE_INT).unwrap_or(false);
                    let scalar = self
                        .types
                        .get(&ty)
                        .and_then(|t| if opcode(t) == OP_TYPE_VECTOR { Some(t[2]) } else { Some(ty) });
                    let is_float = scalar
                        .and_then(|s| self.types.get(&s))
                        .map(|t| opcode(t) == OP_TYPE_FLOAT)
                        .unwrap_or(false);
                    if !is_int && is_float {
                        x[4] = op2;
                        n += 1;
                    }
                }
            }
        }
        n
    }

    pub fn txq_extra_components(&mut self) -> u32 {
        let mut sizes: HashMap<u32, u32> = HashMap::new();
        for x in &self.ins {
            let op = opcode(x);
            if (op == OP_IMAGE_QUERY_SIZE_LOD || op == OP_IMAGE_QUERY_SIZE) && x.len() >= 4 {
                sizes.insert(x[2], x[1]);
            }
        }
        if sizes.is_empty() {
            return 0;
        }
        let Some(i32_ty) = self.find_type(OP_TYPE_INT, &[32, 1]) else {
            return 0;
        };
        let mut bad_indices = Vec::new();
        for (idx, x) in self.ins.iter().enumerate() {
            if opcode(x) == OP_COMPOSITE_EXTRACT && x.len() == 5 && x[1] == i32_ty {
                if let Some(&t) = sizes.get(&x[3]) {
                    let dims = if self.op(t) == Some(OP_TYPE_VECTOR) {
                        self.types[&t][3]
                    } else {
                        1
                    };
                    if x[4] >= dims {
                        bad_indices.push(idx);
                    }
                }
            }
        }
        if bad_indices.is_empty() {
            return 0;
        }
        let one = self.add_const(i32_ty, 1);
        for &idx in &bad_indices {
            let x = &mut self.ins[idx];
            let ty = x[1];
            let res = x[2];
            *x = vec![make_word0(4, OP_COPY_OBJECT), ty, res, one];
        }
        bad_indices.len() as u32
    }

    pub fn buffer_textures_split(&mut self) -> u32 {
        let imgs: HashSet<u32> = self
            .types
            .iter()
            .filter_map(|(&id, x)| {
                if opcode(x) == OP_TYPE_IMAGE && x.len() >= 8 && x[3] == DIM_BUFFER && x[7] == 1 {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        let si: HashMap<u32, u32> = self
            .types
            .iter()
            .filter_map(|(&id, x)| {
                if opcode(x) == OP_TYPE_SAMPLED_IMAGE && x.len() >= 3 && imgs.contains(&x[2]) {
                    Some((id, x[2]))
                } else {
                    None
                }
            })
            .collect();
        if si.is_empty() {
            return 0;
        }
        let loaded: HashSet<u32> = self
            .ins
            .iter()
            .filter_map(|x| {
                let op = opcode(x);
                if (op == OP_LOAD || op == OP_SAMPLED_IMAGE)
                    && x.len() >= 3
                    && si.contains_key(&x[1])
                {
                    Some(x[2])
                } else {
                    None
                }
            })
            .collect();
        let mut body = false;
        for x in &self.ins {
            let op = opcode(x);
            body = body || op == OP_FUNCTION;
            if !body
                || op == OP_LOAD
                || (op == OP_SAMPLED_IMAGE && x.len() >= 2 && si.contains_key(&x[1]))
                || (op == OP_IMAGE && x.len() == 4 && loaded.contains(&x[3]))
            {
                continue;
            }
            if (x.len() > 3 && x[3..].iter().any(|w| loaded.contains(w))
                && !matches!(op, OP_STORE | OP_ACCESS_CHAIN | OP_IN_BOUNDS_ACCESS_CHAIN))
                || (op == OP_STORE && x.len() >= 3 && loaded.contains(&x[2]))
            {
                return 0;
            }
        }
        for x in &mut self.ins {
            let op = opcode(x);
            if matches!(op, OP_TYPE_POINTER | OP_TYPE_ARRAY | OP_TYPE_RUNTIME_ARRAY) {
                let k = if op == OP_TYPE_POINTER { 3 } else { 2 };
                if let Some(&new_t) = si.get(&x[k]) {
                    x[k] = new_t;
                }
            } else if op == OP_LOAD && si.contains_key(&x[1]) {
                x[1] = si[&x[1]];
            } else if op == OP_SAMPLED_IMAGE && si.contains_key(&x[1]) {
                let new_t = si[&x[1]];
                let res = x[2];
                let img = x[3];
                *x = vec![make_word0(4, OP_COPY_OBJECT), new_t, res, img];
            } else if op == OP_IMAGE && x.len() >= 4 && loaded.contains(&x[3]) {
                x[0] = make_word0(x.len(), OP_COPY_OBJECT);
            }
        }
        let used: HashSet<u32> = self
            .ins
            .iter()
            .filter(|x| !matches!(opcode(x), OP_NAME | OP_DECORATE))
            .flat_map(|x| {
                let start = if (19..=39).contains(&opcode(x)) { 2 } else { 1 };
                x[start..].iter().copied()
            })
            .collect();
        self.ins.retain(|x| {
            !(opcode(x) == OP_TYPE_SAMPLED_IMAGE
                && si.contains_key(&x[1])
                && !used.contains(&x[1]))
        });
        self.index();
        si.len() as u32
    }

    pub fn nonuniform_arrays(&mut self) -> u32 {
        let mut arrays = HashSet::new();
        for (&vid, &(ptr, sc)) in &self.vars {
            if sc != SC_UNIFORM_CONSTANT {
                continue;
            }
            let Some((_, t)) = self.pointee(ptr) else {
                continue;
            };
            let (el, _) = self.strip_arrays(t);
            if matches!(self.op(t), Some(OP_TYPE_ARRAY | OP_TYPE_RUNTIME_ARRAY))
                && matches!(
                    self.op(el),
                    Some(OP_TYPE_IMAGE | OP_TYPE_SAMPLER | OP_TYPE_SAMPLED_IMAGE)
                )
            {
                arrays.insert(vid);
            }
        }
        let mut marked = BTreeSet::new();
        let mut storage = false;
        for x in &self.ins {
            let op = opcode(x);
            if matches!(op, OP_ACCESS_CHAIN | OP_IN_BOUNDS_ACCESS_CHAIN)
                && x.len() >= 5
                && arrays.contains(&x[3])
                && x[4..].iter().any(|id| !self.consts.contains_key(id))
            {
                marked.insert(x[2]);
                if let Some((_, t)) = self.pointee(self.vars[&x[3]].0) {
                    let (el, _) = self.strip_arrays(t);
                    if self.op(el) == Some(OP_TYPE_IMAGE) && self.types[&el].get(7) == Some(&2) {
                        storage = true;
                    }
                }
            } else if matches!(op, OP_LOAD | OP_COPY_OBJECT | OP_IMAGE)
                && x.len() >= 4
                && marked.contains(&x[3])
            {
                marked.insert(x[2]);
            } else if op == OP_SAMPLED_IMAGE
                && x.len() >= 5
                && (marked.contains(&x[3]) || marked.contains(&x[4]))
            {
                marked.insert(x[2]);
            }
        }
        marked.retain(|id| {
            !self
                .decos
                .get(id)
                .map(|d| d.contains_key(&DEC_NON_UNIFORM))
                .unwrap_or(false)
        });
        if marked.is_empty() {
            return 0;
        }
        let mut caps_to_add = Vec::new();
        for &c in &[CAP_SHADER_NON_UNIFORM, CAP_SAMPLED_IMAGE_ARRAY_NU] {
            if !self.caps.contains(&c) {
                caps_to_add.push(c);
            }
        }
        if storage && !self.caps.contains(&CAP_STORAGE_IMAGE_ARRAY_NU) {
            caps_to_add.push(CAP_STORAGE_IMAGE_ARRAY_NU);
        }
        let k = self
            .ins
            .iter()
            .rposition(|y| opcode(y) == OP_CAPABILITY)
            .map(|p| p + 1)
            .unwrap_or(0);
        let mut cap_ins: Vec<Vec<u32>> = caps_to_add
            .iter()
            .map(|&c| vec![make_word0(2, OP_CAPABILITY), c])
            .collect();
        if self.header[1] < 0x10500 && !self.exts.iter().any(|e| e == "SPV_EXT_descriptor_indexing")
        {
            let e = lit_string("SPV_EXT_descriptor_indexing");
            let mut ext_inst = vec![make_word0(1 + e.len(), OP_EXTENSION)];
            ext_inst.extend(e);
            cap_ins.push(ext_inst);
        }
        self.ins.splice(k..k, cap_ins);
        let d = self
            .ins
            .iter()
            .rposition(|y| {
                matches!(
                    opcode(y),
                    OP_DECORATE
                        | OP_MEMBER_DECORATE
                        | OP_NAME
                        | 6
                        | 7
                        | 3
                        | 4
                        | OP_EXTENSION
                        | OP_CAPABILITY
                        | OP_EXT_INST_IMPORT
                        | OP_MEMORY_MODEL
                        | OP_ENTRY_POINT
                        | OP_EXECUTION_MODE
                )
            })
            .map(|p| p + 1)
            .unwrap_or(0);
        let deco_ins: Vec<Vec<u32>> = marked
            .iter()
            .map(|&id| vec![make_word0(3, OP_DECORATE), id, DEC_NON_UNIFORM])
            .collect();
        self.ins.splice(d..d, deco_ins);
        self.index();
        marked.len() as u32
    }

    fn ensure_glsl(&mut self) -> u32 {
        for x in &self.ins {
            if opcode(x) == OP_EXT_INST_IMPORT && string_at(x, 2).0 == "GLSL.std.450" {
                return x[1];
            }
        }
        let id = self.new_id();
        let s = lit_string("GLSL.std.450");
        let at = self
            .ins
            .iter()
            .position(|x| opcode(x) == OP_MEMORY_MODEL)
            .unwrap_or(self.ins.len());
        let mut inst = vec![make_word0(2 + s.len(), OP_EXT_INST_IMPORT), id];
        inst.extend(s);
        self.ins.insert(at, inst);
        id
    }

    fn vec4_const(&mut self, f32_ty: u32, vals: [f32; 4]) -> u32 {
        let vt = self.add_type(OP_TYPE_VECTOR, &[f32_ty, 4]);
        let ids: Vec<u32> = vals
            .iter()
            .map(|&v| self.add_const(f32_ty, v.to_bits()))
            .collect();
        for x in &self.ins {
            if opcode(x) == OP_CONSTANT_COMPOSITE
                && x.len() >= 3
                && x[1] == vt
                && x[3..] == ids[..]
            {
                return x[2];
            }
        }
        let c = self.new_id();
        let mut inst = vec![make_word0(7, OP_CONSTANT_COMPOSITE), vt, c];
        inst.extend(ids);
        self.insert_decl(inst);
        c
    }

    pub fn init_outputs(&mut self, extra_locations: &[u32]) -> Vec<u32> {
        let mut outs = self.io(SC_OUTPUT);
        let Some(f32_ty) = self.find_type(OP_TYPE_FLOAT, &[32]) else {
            return Vec::new();
        };
        let one = self.vec4_const(f32_ty, [0.0, 0.0, 0.0, 1.0]);
        let Some(vt) = self.find_type(OP_TYPE_VECTOR, &[f32_ty, 4]) else {
            return Vec::new();
        };
        let mut extra: BTreeSet<u32> = extra_locations.iter().copied().collect();
        for loc in outs.keys() {
            extra.remove(loc);
        }
        let mut added = Vec::new();
        for loc in extra {
            let ptr = self.add_type(OP_TYPE_POINTER, &[SC_OUTPUT, vt]);
            let v = self.new_id();
            self.insert_decl(vec![make_word0(4, OP_VARIABLE), ptr, v, SC_OUTPUT]);
            self.vars.insert(v, (ptr, SC_OUTPUT));
            let at = self
                .ins
                .iter()
                .position(|y| opcode(y) == OP_DECORATE)
                .unwrap_or(self.ins.len());
            self.ins
                .insert(at, vec![make_word0(4, OP_DECORATE), v, DEC_LOCATION, loc]);
            self.decos
                .entry(v)
                .or_default()
                .insert(DEC_LOCATION, vec![loc]);
            let name_words = lit_string(&format!("dm_pad{loc}"));
            let mut name_inst = vec![make_word0(2 + name_words.len(), OP_NAME), v];
            name_inst.extend(name_words);
            self.ins.insert(at, name_inst);
            if let Some(ep) = &mut self.entry {
                ep.push(v);
                ep[0] = make_word0(ep.len(), OP_ENTRY_POINT);
            }
            for x in &mut self.ins {
                if opcode(x) == OP_ENTRY_POINT {
                    x.push(v);
                    x[0] = make_word0(x.len(), OP_ENTRY_POINT);
                    break;
                }
            }
            outs.insert(loc, (v, vt));
            added.push(loc);
        }
        if let Some(fid) = self.entry.as_ref().and_then(|e| e.get(2).copied()) {
            if let Some(k) = self.first_block_insert_point(fid) {
                let stores: Vec<Vec<u32>> = outs
                    .iter()
                    .filter(|(_, &(_, t))| t == vt)
                    .map(|(_, &(v, _))| vec![make_word0(3, OP_STORE), v, one])
                    .collect();
                self.ins.splice(k..k, stores);
            }
        }
        added
    }

    pub fn gl_depth(&mut self, spec_id: u32) -> bool {
        let Some((pos, member)) = self.builtin_var(SC_OUTPUT, BUILTIN_POSITION) else {
            return false;
        };
        let Some(f32_ty) = self.find_type(OP_TYPE_FLOAT, &[32]) else {
            return false;
        };
        let Some(vt) = self.find_type(OP_TYPE_VECTOR, &[f32_ty, 4]) else {
            return false;
        };
        let pvec = self.add_type(OP_TYPE_POINTER, &[SC_OUTPUT, vt]);
        let half = self.add_const(f32_ty, 0x3f00_0000);
        let i32_ty = self.add_type(OP_TYPE_INT, &[32, 1]);
        let midx = member.map(|m| self.add_const(i32_ty, m));
        let glsl = self.ensure_glsl();
        let boolean = self.add_type(OP_TYPE_BOOL, &[]);
        let spec = self.new_id();
        self.insert_decl(vec![make_word0(3, OP_SPEC_CONSTANT_FALSE), boolean, spec]);
        let at = self
            .ins
            .iter()
            .position(|y| opcode(y) == OP_DECORATE)
            .unwrap_or(self.ins.len());
        self.ins
            .insert(at, vec![make_word0(4, OP_DECORATE), spec, DEC_SPEC_ID, spec_id]);
        self.decos
            .entry(spec)
            .or_default()
            .insert(DEC_SPEC_ID, vec![spec_id]);
        let name_words = lit_string("dm_gl_depth");
        let name_at = self
            .ins
            .iter()
            .position(|y| matches!(opcode(y), OP_NAME | OP_DECORATE))
            .unwrap_or(self.ins.len());
        let mut name_inst = vec![make_word0(2 + name_words.len(), OP_NAME), spec];
        name_inst.extend(name_words);
        self.ins.insert(name_at, name_inst);

        let Some(fid) = self.entry.as_ref().and_then(|e| e.get(2).copied()) else {
            return false;
        };
        let Some((a, e)) = self.function_range(fid) else {
            return false;
        };
        let is_gs = self.entry_model() == Some("gs");
        let mut out = Vec::with_capacity(self.ins.len() + 32);
        let mut n = 0usize;
        for k in 0..self.ins.len() {
            let x = self.ins[k].clone();
            let op = opcode(&x);
            let is_stop = if is_gs {
                op == OP_EMIT_VERTEX || op == OP_EMIT_STREAM_VERTEX
            } else {
                op == OP_RETURN || op == OP_RETURN_VALUE
            };
            if k > a && k < e && is_stop {
                let p = if let Some(idx) = midx {
                    let ptr = self.new_id();
                    out.push(vec![make_word0(5, OP_ACCESS_CHAIN), pvec, ptr, pos, idx]);
                    ptr
                } else {
                    pos
                };
                let (v, z, w, wh, z2, z3, v2) = (
                    self.new_id(),
                    self.new_id(),
                    self.new_id(),
                    self.new_id(),
                    self.new_id(),
                    self.new_id(),
                    self.new_id(),
                );
                out.push(vec![make_word0(4, OP_LOAD), vt, v, p]);
                out.push(vec![make_word0(5, OP_COMPOSITE_EXTRACT), f32_ty, z, v, 2]);
                out.push(vec![make_word0(5, OP_COMPOSITE_EXTRACT), f32_ty, w, v, 3]);
                out.push(vec![make_word0(5, OP_FMUL), f32_ty, wh, w, half]);
                out.push(vec![
                    make_word0(8, OP_EXT_INST),
                    f32_ty,
                    z2,
                    glsl,
                    50,
                    z,
                    half,
                    wh,
                ]);
                out.push(vec![make_word0(6, OP_SELECT), f32_ty, z3, spec, z2, z]);
                out.push(vec![make_word0(6, OP_COMPOSITE_INSERT), vt, v2, z3, v, 2]);
                out.push(vec![make_word0(3, OP_STORE), p, v2]);
                n += 1;
            }
            out.push(x.clone());
        }
        self.ins = out;
        n > 0
    }

    pub fn dual_source(&mut self) -> bool {
        let outs = self.io(SC_OUTPUT);
        let keys: Vec<u32> = outs.keys().copied().collect();
        if keys != [0, 1] {
            return false;
        }
        let var = outs[&1].0;
        let mut insert_at = None;
        for (k, x) in self.ins.iter_mut().enumerate() {
            if opcode(x) == OP_DECORATE && x.len() >= 4 && x[1] == var && x[2] == DEC_LOCATION {
                x[3] = 0;
                insert_at = Some(k + 1);
                break;
            }
        }
        if let Some(at) = insert_at {
            self.ins
                .insert(at, vec![make_word0(4, OP_DECORATE), var, DEC_INDEX, 1]);
            self.index();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_precise_minmax() {
        // Minimal SPIR-V: header + GLSL.std.450 + OpTypeFloat 32 + OpExtInst FMin -> NMin
        let glsl = lit_string("GLSL.std.450");
        let mut words = vec![
            MAGIC, 0x0001_0000, 0, 10, 0,
            make_word0(2, OP_CAPABILITY), 1,
        ];
        words.push(make_word0(2 + glsl.len(), OP_EXT_INST_IMPORT));
        words.push(1);
        words.extend(&glsl);
        words.extend([
            make_word0(3, OP_MEMORY_MODEL), 0, 1,
            make_word0(3, OP_TYPE_FLOAT), 2, 32,
            make_word0(7, OP_EXT_INST), 2, 3, 1, 37, 4, 5,
        ]);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut m = SpvModule::parse(&bytes).expect("parse");
        assert_eq!(m.precise_minmax(), 1);
        let out = m.to_bytes();
        let m2 = SpvModule::parse(&out).expect("reparse");
        assert_eq!(m2.ins.last().unwrap()[4], 79); // NMin
    }

    #[test]
    fn strip_unused_caps_drops_float64() {
        let words = vec![
            MAGIC, 0x0001_0000, 0, 10, 0,
            make_word0(2, OP_CAPABILITY), 1,
            make_word0(2, OP_CAPABILITY), CAP_FLOAT64,
            make_word0(3, OP_MEMORY_MODEL), 0, 1,
        ];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut m = SpvModule::parse(&bytes).expect("parse");
        let dropped = m.strip_unused_caps();
        assert_eq!(dropped, vec![CAP_FLOAT64]);
        assert_eq!(m.caps, vec![1]);
    }
}
