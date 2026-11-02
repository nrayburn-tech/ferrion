import org.jspecify.annotations.Nullable;

class TypeUseVariables {
  @Nullable
  private ConcurrentContainer<? extends ObservableResult<? extends ValueT>> resultsAwaitingMerging =
      null;

  private @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
      resultsAwaitingMerging = null;
  @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
      resultsAwaitingMergingAAA = null;
  @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
      resultsAwaitingMergingAAAAAAAAAAAAAAAAA;

  @Deprecated
  @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
      resultsAwaitingMerging = null;

  @Nullable @Deprecated
  ConcurrentContainer<? extends ObservableResult<? extends ValueT>> resultsAwaitingMerging = null;

  @Deprecated @Nullable String shortField;

  void m(
      @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
          resultsAwaitingMergingAtCompletion) {
    @Nullable V[][] array =
        (@Nullable V[][]) new Object[rowIndex.size()][columnIndex.size()][aaaaaaaaaaaa];
    @Deprecated
    @Nullable V[][] array = (@Nullable V[][]) new Object[rowIndex.size()];
    @Nullable
    @Deprecated
    V[][] array = (@Nullable V[][]) new Object[rowIndex.size()];
    @Nullable
    final V[][] array = (@Nullable V[][]) new Object[rowIndex.size()][columnIndex.size()][aaaaaaaa];
    @Nullable final V[][] array = null;
  }

  void n(
      @Nullable @Deprecated
          ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
              resultsAwaitingMergingAtCompletion) {}

  void o(
      @Deprecated
          @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
              resultsAwaitingMergingAtCompletion) {}

  void p(
      final @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
          resultsAwaitingMergingAtCompletion) {}

  void q(
      @Nullable
          final ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
              resultsAwaitingMergingAtCompletion) {}

  void r(
      @Nullable ConcurrentContainer<? extends ObservableResult<? extends ValueT>>
          resultsAwaitingMergingAtCompletion,
      int b) {}
}
