/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::{collections::VecDeque, iter::FusedIterator, mem};

#[cfg(test)]
mod tests;

/// This struct represents a Leaf of a [`DarkTree`],
/// holding this tree node data, along with positional
/// indexes information, based on tree's traversal order.
/// These indexes are only here to enable referencing
/// connected nodes, and are *not* used as pointers by the
/// tree. Creator must ensure they are properly setup.
#[derive(Debug, PartialEq)]
struct DarkLeaf<T> {
    /// Data holded by this leaf
    data: T,
    /// Index showcasing this leaf's position, when all
    /// leafs are in order.
    index: usize,
    /// Index showcasing this leaf's parent tree, when all
    /// leafs are in order. None indicates that this leaf
    /// has no parent.
    parent_index: Option<usize>,
    /// Vector of indexes showcasing this leaf's children
    /// positions, when all leafs are in order. If vector
    /// is empty, it indicates that this leaf has no children.
    children_indexes: Vec<usize>,
}

impl<T> DarkLeaf<T> {
    /// Every leaf is initiated using default indexes.
    fn new(data: T) -> DarkLeaf<T> {
        Self { data, index: 0, parent_index: None, children_indexes: vec![] }
    }
}

/// This struct represents a Tree using DFS post-order traversal,
/// where when we iterate through the tree, we first process tree
/// node's children, and then the node itself, recursively.
/// Based on this, initial tree node (leaf), known as the root,
/// will always show up at the end of iteration.
#[derive(Debug, PartialEq)]
struct DarkTree<T> {
    /// This tree's leaf information, along with its data
    leaf: DarkLeaf<T>,
    /// Vector containing all tree's branches(children tree)
    children: Vec<DarkTree<T>>,
}

impl<T> DarkTree<T> {
    /// Initialize a [`DarkTree`], using provided data to
    /// generate its root.
    fn new(data: T, children: Vec<DarkTree<T>>) -> DarkTree<T> {
        let leaf = DarkLeaf::new(data);
        Self { leaf, children }
    }

    /// Immutably iterate through the tree, using DFS post-order
    /// traversal.
    fn iter(&self) -> DarkTreeIter<'_, T> {
        DarkTreeIter { children: std::slice::from_ref(self), parent: None }
    }

    /// Mutably iterate through the tree, using DFS post-order
    /// traversal.
    fn iter_mut(&mut self) -> DarkTreeIterMut<'_, T> {
        DarkTreeIterMut { children: std::slice::from_mut(self), parent: None, parent_leaf: None }
    }
}

/// Immutable iterator of a [`DarkTree`], performing DFS post-order
/// traversal on the Tree leafs.
struct DarkTreeIter<'a, T> {
    children: &'a [DarkTree<T>],
    parent: Option<Box<DarkTreeIter<'a, T>>>,
}

impl<T> Default for DarkTreeIter<'_, T> {
    fn default() -> Self {
        DarkTreeIter { children: &[], parent: None }
    }
}

impl<'a, T> Iterator for DarkTreeIter<'a, T> {
    type Item = &'a DarkLeaf<T>;

    /// Grab next item iterator visits and return
    /// its immutable reference, or recursively
    /// create and continue iteration on current
    /// leaf's children.
    fn next(&mut self) -> Option<Self::Item> {
        match self.children.first() {
            None => match self.parent.take() {
                Some(parent) => {
                    // Grab parent's leaf
                    *self = *parent;
                    // Its safe to unwrap here as we effectively returned
                    // to this tree after "pushing" it after its children
                    let leaf = &self.children.first().unwrap().leaf;
                    self.children = &self.children[1..];
                    Some(leaf)
                }
                None => None,
            },
            Some(leaf) => {
                // Iterate over tree's children/sub-trees
                *self = DarkTreeIter {
                    children: leaf.children.as_slice(),
                    parent: Some(Box::new(mem::take(self))),
                };
                self.next()
            }
        }
    }
}

impl<T> FusedIterator for DarkTreeIter<'_, T> {}

/// Define fusion iteration behavior, allowing
/// us to use the [`DarkTreeIter`] iterator in
/// loops directly, without using .iter() method
/// of [`DarkTree`].
impl<'a, T> IntoIterator for &'a DarkTree<T> {
    type Item = &'a DarkLeaf<T>;

    type IntoIter = DarkTreeIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Mutable iterator of a [`DarkTree`], performing DFS post-order
/// traversal on the Tree leafs.
struct DarkTreeIterMut<'a, T> {
    children: &'a mut [DarkTree<T>],
    parent: Option<Box<DarkTreeIterMut<'a, T>>>,
    parent_leaf: Option<&'a mut DarkLeaf<T>>,
}

impl<T> Default for DarkTreeIterMut<'_, T> {
    fn default() -> Self {
        DarkTreeIterMut { children: &mut [], parent: None, parent_leaf: None }
    }
}

impl<'a, T> Iterator for DarkTreeIterMut<'a, T> {
    type Item = &'a mut DarkLeaf<T>;

    /// Grab next item iterator visits and return
    /// its mutable reference, or recursively
    /// create and continue iteration on current
    /// leaf's children.
    fn next(&mut self) -> Option<Self::Item> {
        let children = mem::take(&mut self.children);
        match children.split_first_mut() {
            None => match self.parent.take() {
                Some(parent) => {
                    // Grab parent's leaf
                    let parent_leaf = mem::take(&mut self.parent_leaf);
                    *self = *parent;
                    parent_leaf
                }
                None => None,
            },
            Some((first, rest)) => {
                // Setup simplings iteration
                self.children = rest;

                // Iterate over tree's children/sub-trees
                *self = DarkTreeIterMut {
                    children: first.children.as_mut_slice(),
                    parent: Some(Box::new(mem::take(self))),
                    parent_leaf: Some(&mut first.leaf),
                };
                self.next()
            }
        }
    }
}

/// Define fusion iteration behavior, allowing
/// us to use the [`DarkTreeIterMut`] iterator
/// in loops directly, without using .iter_mut()
/// method of [`DarkTree`].
impl<'a, T> IntoIterator for &'a mut DarkTree<T> {
    type Item = &'a mut DarkLeaf<T>;

    type IntoIter = DarkTreeIterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Special iterator of a [`DarkTree`], performing DFS post-order
/// traversal on the Tree leafs, consuming each leaf. Since this
/// iterator consumes the tree, it becomes unusable after it's moved.
struct DarkTreeIntoIter<T> {
    children: VecDeque<DarkTree<T>>,
    parent: Option<Box<DarkTreeIntoIter<T>>>,
}

impl<T> Default for DarkTreeIntoIter<T> {
    fn default() -> Self {
        DarkTreeIntoIter { children: Default::default(), parent: None }
    }
}

impl<T> Iterator for DarkTreeIntoIter<T> {
    type Item = DarkLeaf<T>;

    /// Grab next item iterator visits, and return
    /// its mutable reference, or recursively
    /// create and continue iteration on current
    /// leaf's children.

    /// Move next item iterator visits from the tree
    /// to the iterator consumer, if it has no children.
    /// Otherwise recursively create and continue iteration
    /// on current leaf's children, and moving it after them.
    fn next(&mut self) -> Option<Self::Item> {
        match self.children.pop_front() {
            None => match self.parent.take() {
                Some(parent) => {
                    // Continue iteration on parent's simplings
                    *self = *parent;
                    self.next()
                }
                None => None,
            },
            Some(mut leaf) => {
                // If leaf has no children, return it
                if leaf.children.is_empty() {
                    return Some(leaf.leaf)
                }

                // Push leaf after its children
                let mut children: VecDeque<DarkTree<T>> = leaf.children.into();
                leaf.children = Default::default();
                children.push_back(leaf);

                // Iterate over tree's children/sub-trees
                *self = DarkTreeIntoIter { children, parent: Some(Box::new(mem::take(self))) };
                self.next()
            }
        }
    }
}

impl<T> FusedIterator for DarkTreeIntoIter<T> {}

/// Define fusion iteration behavior, allowing
/// us to use the [`DarkTreeIntoIter`] .into_iter()
/// method, to consume the [`DarkTree`] and iterate
/// over it.
impl<T> IntoIterator for DarkTree<T> {
    type Item = DarkLeaf<T>;

    type IntoIter = DarkTreeIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        let mut children = VecDeque::with_capacity(1);
        children.push_back(self);

        DarkTreeIntoIter { children, parent: None }
    }
}
