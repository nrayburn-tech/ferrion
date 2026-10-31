class Try {
    void m(AutoCloseable r) throws Exception {
        try { } catch (RuntimeException e) { }
        try { } catch (final IllegalStateException | IllegalArgumentException e) { } finally { }
        try { } finally { }
        try (AutoCloseable a = r; var b = r) { }
        try (r; this.field;) { } catch (Exception e) { }
    }

    AutoCloseable field;
}
