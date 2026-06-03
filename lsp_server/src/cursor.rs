use rustc_hash::FxHashMap;
use tree_sitter::Range;


pub(crate) type ScopeId = usize;

#[allow(unused)]
#[derive(Debug,Clone, PartialEq, Eq)]
pub(crate) enum ScopeType {
    TopLevel,
    Contract,
    Function,
    Block,
}


#[derive(Debug,Default,Clone, PartialEq, Eq)]
pub(crate) struct Scope {//@NOTE should scope be per file or for entire workspace?? rename to ScopeTree
    parent: FxHashMap<ScopeId/*child*/,ScopeId/*parent*/>,
    sub_scopes: FxHashMap<ScopeId/*scope */,Vec<ScopeId>/*children */>,//@note 💡 with inheritance we can have the child contract scope inherit the subscopes of the parent contract, this way the scoping rules are not violated
    data: FxHashMap<ScopeId/*scope*/,(ScopeType,Range)/*byte range*/>,
}

#[allow(unused)]
impl Scope {
    //@NOTE what if scope id carries the scope information in itself i.e. 0 -> 0_2 -> 0_2_5 -> 0_2_5_1. we can immediately tell the parent scopes and heirachy just from the id
    pub fn new() -> Self {
        Self {
            sub_scopes: FxHashMap::default(),
            parent: FxHashMap::default(),
            data: FxHashMap::default()
        }
    }

    pub(crate) fn build(self) -> ScopeBuilder {
        ScopeBuilder::new(self)
    }

    pub(crate) fn walk(&self) -> ScopeCursor<'_> {
        ScopeCursor::new(self)
    }


    //@NOTE scope 0 has no range since it includes everything top level including imports
    pub fn get_range(&self, scope: ScopeId) -> Option<Range> {
        self.data.get(&scope).map(|(_, range)| range.clone())
    }



    fn encloses(&self, enclosing_scope: ScopeId, target: Range) -> bool {
        self.data.get(&enclosing_scope).map(|(_,range)| {
            range.start_byte <= target.start_byte && target.end_byte <= range.end_byte
        }).unwrap_or(false)
    }

    
    fn find_enclosing_scope(&self,current: ScopeId, target: Range) -> Option<ScopeId> {
        let children = self.sub_scopes.get(&current)?;
    
        // first child with start_byte > byte
        let idx = children.partition_point(|cid| {//binary search sibling scopes are ordered by start_byte
            self.data
                .get(cid)
                .map(|(_,r)| r.start_byte <= target.start_byte)
                .unwrap_or(false)
        });
    
        if idx == 0 {
            return None;
        }
        
        let candidate = children[idx-1];
        self.encloses(candidate, target).then_some(candidate)
    }
    
    pub(crate) fn enclosing_scope(&self, target: Range) -> ScopeId {
        let mut cur_enclosing_scope = 0;

        while let Some(nxt_enclosing_scope) = self.find_enclosing_scope(cur_enclosing_scope,target) {
            cur_enclosing_scope = nxt_enclosing_scope;
        }
        
        cur_enclosing_scope
    }
}


#[allow(unused)]
pub(crate) trait ScopeNavigator {

    fn scope(&self) -> &Scope;
    fn current(&self) -> ScopeId;
    fn set_current(&mut self, scope: ScopeId);
    fn nxt_idx(&self) -> usize;
    fn set_nxt_idx(&mut self, idx: usize);

    fn get_parent(&self) -> Option<ScopeId> {
        self.scope().parent.get(&self.current()).copied()
    }

    fn to_parent(&mut self) -> bool {
        if let Some(parent) = self.get_parent() {
            self.set_current(parent);
            self.set_nxt_idx(0);
            true
        } else {
            false
        }
    }

    fn to_first_child(&mut self) -> bool {
        if let Some(sub_scopes) = self.scope().sub_scopes.get(&self.current()) && !sub_scopes.is_empty()  {
            self.set_current(sub_scopes[0]);
            self.set_nxt_idx(1);
            true
        } else {
            false
        }
    }

    fn to_next_sibling(&mut self) -> bool {
        if let Some(parent) = self.get_parent() 
        && let Some(sub_scopes) = self.scope().sub_scopes.get(&parent) 
        && self.nxt_idx() < sub_scopes.len() {
            self.set_current(sub_scopes[self.nxt_idx()]);
            self.set_nxt_idx(self.nxt_idx() + 1);
            true
        } else {
            false
        }
    }

    fn is_top_level(&self) -> bool {
        self.current() == 0
    }

    ///returns the scope to top level
    fn reset(&mut self) -> &mut Self {
        self.set_current(0);
        self.set_nxt_idx(0);
        self
    }

    fn to_scope(&mut self, scope: ScopeId) -> &mut Self {
        //@TODO asset scope in not OOB ie < nxt_scope
        self.set_current(scope);
        self.set_nxt_idx(0);
        self
    }

    fn depth(&self) -> usize {
        let mut depth = 0;
        let mut current = self.current();
        
        while let Some(parent) = self.scope().parent.get(&current) {
            depth += 1;
            current = *parent;
        }
        
        depth
    }

    fn get_depth(&self, id: ScopeId) -> usize {
        let mut depth = 0;
        let mut current = id;
        
        while let Some(parent) = self.scope().parent.get(&current) {
            depth += 1;
            current = *parent;
        }
        
        depth
    }

    fn encloses(&self, target: ScopeId) -> bool {
        self.scope().data.get(&self.current()).map(|(_,range)| {
            if let Some(target_range) = self.scope().data.get(&target).map(|(_,r)| r) {
                range.start_byte <= target_range.start_byte && target_range.end_byte <= range.end_byte
            } else {
                false
            }
        }).unwrap_or(false)
    }

    fn to_enclosing_scope(&mut self, target: Range) {
        let mut cur_enclosing_scope = 0;

        while let Some(nxt_enclosing_scope) = self.scope().find_enclosing_scope(cur_enclosing_scope,target) {
            cur_enclosing_scope = nxt_enclosing_scope;
        }
        
        self.set_current(cur_enclosing_scope);
    }
}


#[allow(unused)]
pub(crate) struct ScopeBuilder {
    current: ScopeId,
    nxt_idx: usize,
    nxt_scope: ScopeId,
    scope: Scope,
}

impl ScopeNavigator for ScopeBuilder {
    fn scope(&self) -> &Scope {
        &self.scope
    }
    
    fn current(&self) -> ScopeId {
        self.current
    }
    
    fn set_current(&mut self, scope: ScopeId) {
        self.current = scope;
    }
    
    fn nxt_idx(&self) -> usize {
        self.nxt_idx
    }
    
    fn set_nxt_idx(&mut self, idx: usize) {
        self.nxt_idx = idx;
    }
}

impl ScopeBuilder {
    pub fn new(scope: Scope) -> Self {
        // If you ever build on pre-populated scope trees, compute a real max here.
        let nxt_scope = scope.data.keys().copied().max().unwrap_or(0);
        Self {
            current: 0,
            nxt_scope: nxt_scope + 1,
            nxt_idx: 0,
            scope,
        }
    }

    pub fn finish(self) -> Scope {
        self.scope
    }

    pub fn to_next(&mut self, range: Range, typ: ScopeType) {
        let sub_scopes = self.scope.sub_scopes.entry(self.current).or_default();
        //@NOTE test to ensure scope siblings are ordered;
        // assert!(sub_scopes.is_empty() || self.range[&sub_scopes[sub_scopes.len()-1]].start_byte <= range.start_byte);
        sub_scopes.push(self.nxt_scope);
        
        self.scope.parent.insert(self.nxt_scope, self.current);
        self.scope.data.insert(self.nxt_scope, (typ, range));

        self.current = self.nxt_scope;
        self.nxt_scope += 1; 
    }
}


pub(crate) struct ScopeCursor<'scope> {
    current: ScopeId,
    nxt_idx: usize,
    scope: &'scope Scope,
}

impl<'scope> ScopeNavigator for ScopeCursor<'scope> {
    fn scope(&self) -> &Scope {
        self.scope
    }
    
    fn current(&self) -> ScopeId {
        self.current
    }
    
    fn set_current(&mut self, scope: ScopeId) {
        self.current = scope;
    }
    
    fn nxt_idx(&self) -> usize {
        self.nxt_idx
    }
    
    fn set_nxt_idx(&mut self, idx: usize) {
        self.nxt_idx = idx;
    }
}

impl<'scope> ScopeCursor<'scope> {
    pub fn new(scope: &'scope Scope) -> Self {
        Self {
            current: 0,
            nxt_idx: 0,
            scope,
        }
    }
}


