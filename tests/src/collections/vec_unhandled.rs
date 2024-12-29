// None of those functions are monomorphic, so they would be rejected right away.

fn splice<R, I>(
  vec: &mut Vec<usize>,
  range: R,
  replace_with: I,
) -> Splice<'_, <I as IntoIterator>::IntoIter, Global>
where
  R: RangeBounds<usize>,
  I: IntoIterator<Item = usize>;

fn concat<Item>(&self) -> <[T] as Concat<Item>>::Output 
where
    [T]: Concat<Item>, Item: ?Sized;

fn join<Separator>(
    &self,
    sep: Separator
) -> <[T] as Join<Separator>>::Output 
where
    [T]: Join<Separator>;

fn rsplit<F>(&self, pred: F) -> RSplit<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn rsplit_mut<F>(&mut self, pred: F) -> RSplitMut<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn rsplitn<F>(&self, n: usize, pred: F) -> RSplitN<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn rsplitn_mut<F>(&mut self, n: usize, pred: F) -> RSplitNMut<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn split<F>(&self, pred: F) -> Split<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn split_inclusive<F>(&self, pred: F) -> SplitInclusive<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn split_inclusive_mut<F>(&mut self, pred: F) -> SplitInclusiveMut<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn split_mut<F>(&mut self, pred: F) -> SplitMut<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn splitn<F>(&self, n: usize, pred: F) -> SplitN<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn splitn_mut<F>(&mut self, n: usize, pred: F) -> SplitNMut<'_, T, F> 
where
    F: FnMut(&T) -> bool;

fn strip_prefix<P>(&self, prefix: &P) -> Option<&[T]>
where
    P: SlicePattern<Item = T> + ?Sized,
    T: PartialEq<T>;

fn strip_suffix<P>(&self, suffix: &P) -> Option<&[T]>
where
    P: SlicePattern<Item = T> + ?Sized,
    T: PartialEq<T>;