class Fftw < Formula
  desc "C routines to compute the Discrete Fourier Transform"
  homepage "https://fftw.org"
  url "https://fftw.org/fftw-3.3.11.tar.gz"
  sha256 "5630c24cdeb33b131612f7eb4b1a9934234754f9f388ff8617458d0be6f239a1"
  license all_of: ["GPL-2.0-or-later", "BSD-2-Clause"]
  compatibility_version 1

  livecheck do
    url :homepage
    regex(%r{latest official release.*? <b>v?(\d+(?:\.\d+)+)</b>}i)
  end

  depends_on "open-mpi" => :build

  on_macos do
    depends_on "libomp"
  end

  def install
    if OS.mac?
      ENV["OPENMP_CFLAGS"] = "-Xpreprocessor -fopenmp"
      ENV.append "LDFLAGS", "-lomp -Wl,-dead_strip_dylibs"
    end

    # FFTW supports runtime detection of CPU capabilities, so it is safe to
    # use with --enable-avx and the code will still run on all CPUs
    ENV.runtime_cpu_detection
    simd_args = []
    simd_args += %w[--enable-sse2 --enable-avx --enable-avx2] if Hardware::CPU.intel?

    common_args = %w[
      --enable-shared
      --enable-threads
      --enable-mpi
      --enable-openmp
    ]
    # Not default yet: https://github.com/FFTW/fftw3/pull/315#issuecomment-2630106315
    common_args << "--enable-armv8-cntvct-el0" if Hardware::CPU.arm64?

    # enable-sse2, enable-avx and enable-avx2 work for both single and double precision.
    # long-double precision has no SIMD optimization available.
    {
      "single"      => ["--enable-single", *simd_args],
      "double"      => simd_args,
      "long-double" => ["--enable-long-double"],
    }.each do |precision, args|
      mkdir "build-#{precision}" do
        system "../configure", *args, *common_args, *std_configure_args
        system "make", "install"
      end
    end
  end

  test do
    # Adapted from the sample usage provided in the documentation:
    # https://www.fftw.org/fftw3_doc/Complex-One_002dDimensional-DFTs.html
    (testpath/"fftw.c").write <<~C
      #include <fftw3.h>
      int main(int argc, char* *argv)
      {
          fftw_complex *in, *out;
          fftw_plan p;
          long N = 1;
          in = (fftw_complex*) fftw_malloc(sizeof(fftw_complex) * N);
          out = (fftw_complex*) fftw_malloc(sizeof(fftw_complex) * N);
          p = fftw_plan_dft_1d(N, in, out, FFTW_FORWARD, FFTW_ESTIMATE);
          fftw_execute(p); /* repeat as needed */
          fftw_destroy_plan(p);
          fftw_free(in); fftw_free(out);
          return 0;
      }
    C

    system ENV.cc, "-o", "fftw", "fftw.c", "-L#{lib}", "-lfftw3"
    system "./fftw"
  end
end
