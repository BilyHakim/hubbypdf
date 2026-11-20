# HubbyPDF

Pembaca dan penyunting PDF lokal untuk Windows x64. HubbyPDF menggunakan Rust, kontrol Win32, Direct2D untuk menampilkan halaman, dan PDFium untuk membaca serta menyimpan PDF. Aplikasi merender satu halaman dan membatasi sisi terpanjang bitmap hingga 4.096 piksel dan total sekitar 9,5 juta piksel.

## Membuka aplikasi

Jalankan `target/HubbyPDF/HubbyPDF.exe`. Jika dipindahkan ke komputer lain, salin seluruh folder `target/HubbyPDF` agar DLL dan lisensi PDFium ikut terbawa.

- Klik **Tema** di kanan atas atau pilih **Tampilan → Tema gelap**. HubbyPDF mengingat pilihan tema dan menampilkan PDF dengan warna aslinya.
- Pilih **Bahasa → Bahasa Indonesia** atau **Bahasa → English**. HubbyPDF mengingat pilihan bahasa.
- **File → Buka PDF** atau **Ctrl+O**. Anda juga bisa memberikan path PDF sebagai argumen.
- Pilih zoom **25%–500%** atau **Pas jendela** di kanan bawah. **100%** berarti ukuran aktual PDF pada skala Windows. Gunakan **Page Up / Page Down** untuk pindah halaman, **Ctrl+roda mouse** untuk zoom, dan **tombol panah** untuk menggeser halaman yang lebih besar dari area baca.
- Klik nomor halaman di kiri bawah untuk membuka daftar dengan mini preview, lalu pilih halaman yang ingin dibuka.
- Klik **Teks**, lalu klik posisi pada halaman. Ketik teks Latin dasar berukuran 14 pt dan tekan **Enter** untuk menempatkannya. **Esc** membatalkan teks yang masih diketik.
- Pilih **Warna** setelah mengaktifkan **Teks**, **Sketsa**, **Tanda tangan**, atau **Bentuk**. Tiap alat mengingat warna terakhirnya selama aplikasi terbuka.
- Buka **Bentuk**, pilih persegi panjang, elips, garis, atau panah, lalu seret di halaman. Untuk panah, titik lepas mouse menentukan letak ujung panah.
- **Sorot**, **Sketsa**, dan **Tanda tangan** menggunakan gerakan seret. Dengan **Impor PNG**, seret secara diagonal untuk mengatur posisi dan ukuran. Gambar yang sisi terpanjangnya melebihi 1.024 piksel diperkecil saat impor.
- Klik **Pilih**, lalu seret teks tambahan, sorotan, sketsa, atau tanda tangan yang dibuat dalam sesi ini. Goresan yang dibuat dalam satu penggunaan mode Tanda tangan ikut digeser bersama. Untuk memindahkan objek lagi, tetap gunakan dokumen yang sedang terbuka. Salinan PDF menyimpan posisi akhir sebagai konten halaman.
- Dalam mode **Pilih**, klik kolom teks, dropdown, atau checkbox pada formulir AcroForm. Tekan **Enter** untuk menerapkan teks atau **Esc** untuk membatalkannya. **Ctrl+Shift+S** menyimpan salinan dengan kolom yang tetap bisa diisi. Kalkulasi dan format otomatis yang bergantung pada JavaScript PDF belum didukung.
- Klik **Hapus**, lalu klik teks tambahan, sorotan, goresan sketsa, atau tanda tangan yang dibuat pada dokumen yang sedang terbuka. Klik salah satu goresan tanda tangan untuk menghapus seluruh tanda tangan. **Ctrl+Z** mengembalikan objek yang terakhir dihapus.
- **Ctrl+Z** membatalkan edit terakhir. **Esc** kembali ke mode Pilih. **Ctrl+Shift+S** menyimpan hasil sebagai file PDF terpisah.

Versi awal mendukung penambahan objek pada halaman. Penggantian teks asli, OCR, pembukaan PDF yang membutuhkan kata sandi, dan tanda tangan digital berbasis sertifikat berada di luar cakupan versi ini.

## Build

Gunakan Rust stable dan Visual Studio Build Tools dengan komponen **Desktop development with C++** dan Windows SDK. Dari folder proyek, jalankan:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File ./build.ps1 -Test
```

Skrip mengunduh PDFium Windows x64 pada build pertama, memeriksa SHA-256 arsip, dan menyalin aplikasi serta dependensi runtime ke `target/HubbyPDF`. Opsi `-Test` menjalankan pengujian PDF nyata yang memverifikasi render, teks, sorotan, sketsa, tanda tangan, simpan salinan, dan undo.



