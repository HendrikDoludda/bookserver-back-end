-- A cover row names a file in the cover folder, whose place the server is given at start; a path
-- relative to the working directory would break wherever the server starts elsewhere.
UPDATE books SET cover_image = substr(cover_image, length('./cover_images/') + 1)
    WHERE substr(cover_image, 1, length('./cover_images/')) = './cover_images/';
UPDATE series SET cover_image = substr(cover_image, length('./cover_images/') + 1)
    WHERE substr(cover_image, 1, length('./cover_images/')) = './cover_images/';
UPDATE library SET cover_image = substr(cover_image, length('./cover_images/') + 1)
    WHERE substr(cover_image, 1, length('./cover_images/')) = './cover_images/';

-- A book whose file couldn't be hashed got `./cover_images.<ext>`: one file beside the folder,
-- shared by every such book. It isn't that book's cover, so the next scan tries again.
UPDATE books SET cover_image = NULL
    WHERE substr(cover_image, 1, length('./cover_images.')) = './cover_images.';
